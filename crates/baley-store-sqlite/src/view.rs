//! Views in the SQLite adapter (design 0001, The storage port; Views and
//! projectors; Physical schema; EVD-R9).
//!
//! Each declared view is one table, `v_<view>`, with the project, the
//! generation, one column per key field (`k_<field>`), one per index field
//! (`i_<field>`, taken from the document body's top-level fields), the
//! provenance and the document. Each declared index is a SQL index on the
//! project, the generation, its fields in their orders, then the key, so
//! every order is total. `find` pages through that index with seeks bounded
//! in SQL, never a scan or a sort.

use std::collections::{BTreeMap, BTreeSet};

use baley_store::{
    Change, Cursor, DocKey, Document, FieldKind, IndexQuery, IndexSpec, KeyValue, Order, Page,
    ProjectId, Refusal, StoreError, ViewSpec, Views,
};
use rusqlite::types::Value as SqlValue;
use rusqlite::{Connection, OptionalExtension, Row, params_from_iter};

use crate::store::{SqliteStore, sql};

/// The longest view, index or field name accepted.
const MAX_NAME: usize = 48;

/// The declared views, checked once when the store opens.
pub(crate) struct ViewSet(BTreeMap<String, ViewTable>);

/// One checked view and the SQL names it becomes.
pub(crate) struct ViewTable {
    spec: ViewSpec,
    table: String,
    /// Each distinct index field once, in first-declared order.
    index_columns: Vec<(String, FieldKind)>,
}

/// One column of a page's order after the equality prefix.
#[derive(Debug, Clone, PartialEq, Eq)]
struct OrderColumn {
    column: String,
    kind: FieldKind,
    order: Order,
}

/// One statement `find` runs: its SQL and the cursor values it binds after
/// the project, the generation and the equality values. A limit is bound
/// last.
#[derive(Debug, Clone, PartialEq)]
struct Seek {
    sql: String,
    bound: Vec<SqlValue>,
}

impl Seek {
    /// Every value the statement binds, in order.
    fn params(
        &self,
        project: &ProjectId,
        generation: i64,
        equals: &[SqlValue],
        limit: usize,
    ) -> Vec<SqlValue> {
        let mut params = vec![
            SqlValue::Text(project.0.clone()),
            SqlValue::Integer(generation),
        ];
        params.extend(equals.iter().cloned());
        params.extend(self.bound.iter().cloned());
        // A page is at most a u32 bound plus one, so it fits.
        params.push(SqlValue::Integer(limit as i64));
        params
    }
}

/// A row `find` read: the document, and its values in the page's order.
struct Found {
    document: Document,
    position: Vec<KeyValue>,
}

impl ViewSet {
    /// Checks every spec. Names become SQL identifiers, so anything that is
    /// not a plain lowercase identifier is refused rather than quoted.
    pub(crate) fn new(specs: &[ViewSpec]) -> Result<Self, StoreError> {
        let mut views = BTreeMap::new();
        for spec in specs {
            let table = ViewTable::new(spec).map_err(|reason| malformed(&spec.name, reason))?;
            if views.insert(spec.name.clone(), table).is_some() {
                return Err(malformed(&spec.name, "the view is declared twice".into()));
            }
        }
        Ok(Self(views))
    }

    /// Whether no view is declared, so open has nothing to create.
    pub(crate) fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Creates each view's table and indexes where missing.
    pub(crate) fn create(&self, tx: &rusqlite::Transaction<'_>) -> Result<(), StoreError> {
        for table in self.0.values() {
            tx.execute_batch(&table.create_sql()).map_err(sql)?;
        }
        Ok(())
    }

    /// The declared view of that name.
    pub(crate) fn table(&self, view: &str) -> Result<&ViewTable, StoreError> {
        self.0
            .get(view)
            .ok_or_else(|| StoreError::Refused(Refusal::UnknownView(view.to_owned())))
    }
}

impl ViewTable {
    /// Checks one spec; the reason names what is wrong.
    fn new(spec: &ViewSpec) -> Result<Self, String> {
        let unsafe_name = |what: &str, name: &str| {
            format!(
                "{what} {name:?} is not a lowercase identifier of at most {MAX_NAME} characters"
            )
        };
        if !safe_identifier(&spec.name) {
            return Err(unsafe_name("the view name", &spec.name));
        }
        if spec.page_bound == 0 {
            return Err("the page bound is zero".into());
        }
        let mut keys = BTreeSet::new();
        for field in &spec.key {
            if !safe_identifier(&field.name) {
                return Err(unsafe_name("the key field", &field.name));
            }
            if !keys.insert(&field.name) {
                return Err(format!("the key field {} is declared twice", field.name));
            }
        }
        let mut index_names = BTreeSet::new();
        let mut index_columns: Vec<(String, FieldKind)> = Vec::new();
        for index in &spec.indexes {
            if !safe_identifier(&index.name) {
                return Err(unsafe_name("the index", &index.name));
            }
            if !index_names.insert(&index.name) {
                return Err(format!("the index {} is declared twice", index.name));
            }
            if index.fields.is_empty() {
                return Err(format!("the index {} has no fields", index.name));
            }
            let mut fields = BTreeSet::new();
            for field in &index.fields {
                if !safe_identifier(&field.name) {
                    return Err(unsafe_name("the index field", &field.name));
                }
                if !fields.insert(&field.name) {
                    return Err(format!(
                        "the index {} names {} twice",
                        index.name, field.name
                    ));
                }
                match index_columns.iter().find(|(name, _)| *name == field.name) {
                    Some((_, kind)) if *kind != field.kind => {
                        return Err(format!(
                            "the index field {} is declared with two kinds",
                            field.name
                        ));
                    }
                    Some(_) => {}
                    None => index_columns.push((field.name.clone(), field.kind)),
                }
            }
        }
        Ok(Self {
            spec: spec.clone(),
            table: format!("v_{}", spec.name),
            index_columns,
        })
    }

    /// The table and its indexes, each created only if missing. Every name
    /// in it passed `safe_identifier`.
    fn create_sql(&self) -> String {
        let mut columns = vec![
            "project_id TEXT NOT NULL REFERENCES project(project_id)".to_owned(),
            "generation INTEGER NOT NULL".to_owned(),
        ];
        for field in &self.spec.key {
            columns.push(format!(
                "k_{} {} NOT NULL",
                field.name,
                sql_type(field.kind)
            ));
        }
        for (name, kind) in &self.index_columns {
            columns.push(format!("i_{name} {} NOT NULL", sql_type(*kind)));
        }
        columns.push("produced_seq INTEGER NOT NULL".to_owned());
        columns.push("projector_version INTEGER NOT NULL".to_owned());
        columns.push("doc_json TEXT NOT NULL".to_owned());
        let mut primary = vec!["project_id".to_owned(), "generation".to_owned()];
        primary.extend(self.key_columns());
        let mut sql = format!(
            "CREATE TABLE IF NOT EXISTS {} (\n  {},\n  PRIMARY KEY ({})\n) STRICT, WITHOUT ROWID;\n",
            self.table,
            columns.join(",\n  "),
            primary.join(", ")
        );
        for index in &self.spec.indexes {
            let mut columns = vec!["project_id".to_owned(), "generation".to_owned()];
            columns.extend(
                index
                    .fields
                    .iter()
                    .map(|field| format!("i_{} {}", field.name, direction(field.order))),
            );
            columns.extend(self.key_columns().map(|column| format!("{column} ASC")));
            sql.push_str(&format!(
                "CREATE INDEX IF NOT EXISTS {} ON {} ({});\n",
                self.index_name(index),
                self.table,
                columns.join(", ")
            ));
        }
        sql
    }

    /// `v_<view>__<index>`. Names hold no double underscore, so no two
    /// (view, index) pairs share one.
    fn index_name(&self, index: &IndexSpec) -> String {
        format!("{}__{}", self.table, index.name)
    }

    fn key_columns(&self) -> impl Iterator<Item = String> + '_ {
        self.spec
            .key
            .iter()
            .map(|field| format!("k_{}", field.name))
    }

    fn index(&self, name: &str) -> Result<&IndexSpec, StoreError> {
        self.spec
            .indexes
            .iter()
            .find(|index| index.name == name)
            .ok_or_else(|| {
                StoreError::Refused(Refusal::UndeclaredIndex {
                    view: self.spec.name.clone(),
                    index: name.to_owned(),
                })
            })
    }

    /// The key as SQL values, refused unless it has the declared fields'
    /// number and kinds.
    fn key_values(&self, key: &DocKey) -> Result<Vec<SqlValue>, StoreError> {
        let kinds: Vec<FieldKind> = self.spec.key.iter().map(|field| field.kind).collect();
        fitted(&key.0, &kinds).ok_or_else(|| {
            malformed(
                &self.spec.name,
                format!(
                    "the key has {} values that do not fit its {} declared fields",
                    key.0.len(),
                    kinds.len()
                ),
            )
        })
    }

    /// The equality values, refused unless they fit a leading prefix of the
    /// index's fields.
    fn equals_values(
        &self,
        index: &IndexSpec,
        equals: &[KeyValue],
    ) -> Result<Vec<SqlValue>, StoreError> {
        let kinds: Vec<FieldKind> = index.fields.iter().map(|field| field.kind).collect();
        kinds
            .get(..equals.len())
            .and_then(|prefix| fitted(equals, prefix))
            .ok_or_else(|| {
                malformed(
                    &self.spec.name,
                    format!(
                        "{} equality values do not fit a prefix of the index {}",
                        equals.len(),
                        index.name
                    ),
                )
            })
    }

    /// The columns a page is ordered by once the first `fixed` index fields
    /// are held equal: the rest of the index in its orders, then the key
    /// ascending.
    fn order_after(&self, index: &IndexSpec, fixed: usize) -> Vec<OrderColumn> {
        let rest = index.fields.iter().skip(fixed).map(|field| OrderColumn {
            column: format!("i_{}", field.name),
            kind: field.kind,
            order: field.order,
        });
        let key = self.spec.key.iter().map(|field| OrderColumn {
            column: format!("k_{}", field.name),
            kind: field.kind,
            order: Order::Ascending,
        });
        rest.chain(key).collect()
    }

    /// The statements that read a page, in page order. From the start that
    /// is one seek. After a position p over order columns c0..cm, the rows
    /// strictly after p are, in order: those equal to p on c0..c(m-1) and
    /// past it on cm, then those equal on c0..c(m-2) and past it on c(m-1),
    /// and so on down to c0. Each is one index seek, an equality prefix and
    /// one range, so a page never walks rows it does not return and never
    /// sorts. A single row-value comparison cannot express mixed orders.
    fn seeks(&self, index: &IndexSpec, fixed: usize, after: Option<&[KeyValue]>) -> Vec<Seek> {
        let order = self.order_after(index, fixed);
        let mut columns: Vec<String> = vec![
            "produced_seq".into(),
            "projector_version".into(),
            "doc_json".into(),
        ];
        columns.extend(order.iter().map(|column| column.column.clone()));
        let mut prefix = String::from("project_id = ? AND generation = ?");
        for field in index.fields.iter().take(fixed) {
            prefix.push_str(&format!(" AND i_{} = ?", field.name));
        }
        let mut ordering: Vec<String> = index
            .fields
            .iter()
            .map(|field| format!("i_{} {}", field.name, direction(field.order)))
            .collect();
        ordering.extend(self.key_columns().map(|column| format!("{column} ASC")));
        let statement = |condition: &str| {
            format!(
                "SELECT {} FROM {} INDEXED BY {} WHERE {prefix}{condition} ORDER BY {} LIMIT ?",
                columns.join(", "),
                self.table,
                self.index_name(index),
                ordering.join(", ")
            )
        };
        let Some(after) = after else {
            return vec![Seek {
                sql: statement(""),
                bound: Vec::new(),
            }];
        };
        (0..order.len())
            .rev()
            .map(|last| {
                let mut condition = String::new();
                for column in &order[..last] {
                    condition.push_str(&format!(" AND {} = ?", column.column));
                }
                let past = match order[last].order {
                    Order::Ascending => ">",
                    Order::Descending => "<",
                };
                condition.push_str(&format!(" AND {} {past} ?", order[last].column));
                Seek {
                    sql: statement(&condition),
                    bound: after[..=last].iter().map(sql_value).collect(),
                }
            })
            .collect()
    }

    /// Reads one row of a seek: the provenance, the document, then the
    /// order columns, whose last values are the key.
    fn found(&self, row: &Row<'_>, order: &[OrderColumn]) -> rusqlite::Result<Found> {
        let mut position = Vec::with_capacity(order.len());
        for (at, column) in order.iter().enumerate() {
            position.push(key_value(row, at + 3, column.kind)?);
        }
        let key = DocKey(position[position.len() - self.spec.key.len()..].to_vec());
        Ok(Found {
            document: document(row, key)?,
            position,
        })
    }
}

/// The rule for every declared name: a lowercase ASCII letter, then lowercase letters and
/// digits in runs joined by single underscores.
fn safe_identifier(name: &str) -> bool {
    name.len() <= MAX_NAME
        && name.starts_with(|first: char| first.is_ascii_lowercase())
        && name.split('_').all(|run| {
            !run.is_empty()
                && run
                    .bytes()
                    .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit())
        })
}

/// The values as SQL values when they match `kinds` one for one.
fn fitted(values: &[KeyValue], kinds: &[FieldKind]) -> Option<Vec<SqlValue>> {
    if values.len() != kinds.len() {
        return None;
    }
    values
        .iter()
        .zip(kinds)
        .map(|(value, kind)| match (value, kind) {
            (KeyValue::Text(_), FieldKind::Text) | (KeyValue::Integer(_), FieldKind::Integer) => {
                Some(sql_value(value))
            }
            _ => None,
        })
        .collect()
}

fn sql_value(value: &KeyValue) -> SqlValue {
    match value {
        KeyValue::Text(text) => SqlValue::Text(text.clone()),
        KeyValue::Integer(number) => SqlValue::Integer(*number),
    }
}

fn sql_type(kind: FieldKind) -> &'static str {
    match kind {
        FieldKind::Text => "TEXT",
        FieldKind::Integer => "INTEGER",
    }
}

fn direction(order: Order) -> &'static str {
    match order {
        Order::Ascending => "ASC",
        Order::Descending => "DESC",
    }
}

fn malformed(view: &str, reason: String) -> StoreError {
    StoreError::Refused(Refusal::MalformedKey {
        view: view.to_owned(),
        reason,
    })
}

fn key_value(row: &Row<'_>, at: usize, kind: FieldKind) -> rusqlite::Result<KeyValue> {
    Ok(match kind {
        FieldKind::Text => KeyValue::Text(row.get(at)?),
        FieldKind::Integer => KeyValue::Integer(row.get(at)?),
    })
}

/// A document from a row that starts with its provenance and JSON.
fn document(row: &Row<'_>, key: DocKey) -> rusqlite::Result<Document> {
    let produced_seq: i64 = row.get(0)?;
    let produced_seq = u64::try_from(produced_seq)
        .map_err(|_| rusqlite::Error::IntegralValueOutOfRange(0, produced_seq))?;
    let projector_version: u32 = row.get(1)?;
    let json: String = row.get(2)?;
    let body = json.parse().map_err(|error| {
        rusqlite::Error::FromSqlConversionFailure(2, rusqlite::types::Type::Text, Box::new(error))
    })?;
    Ok(Document {
        key,
        produced_seq,
        projector_version,
        body,
    })
}

/// The project's live generation. A project with no `project_gen` row has
/// never been rebuilt and reads generation 0.
fn live_generation(conn: &Connection, project: &ProjectId) -> Result<i64, StoreError> {
    conn.query_row(
        "SELECT g.live_gen FROM project p
           LEFT JOIN project_gen g ON g.project_id = p.project_id
          WHERE p.project_id = ?1",
        [&project.0],
        |row| row.get::<_, Option<i64>>(0),
    )
    .optional()
    .map_err(sql)?
    .map(|generation| generation.unwrap_or(0))
    .ok_or_else(|| StoreError::Refused(Refusal::UnknownProject(project.clone())))
}

/// Puts or deletes one document in the project's live generation, stamped
/// with the event that produced it and the view's projector version. Index
/// columns come from the body's top-level fields; a body missing one, or
/// holding the wrong kind, is refused, so every index order stays total.
#[cfg_attr(
    not(test),
    expect(dead_code, reason = "the projectors of task 7 are its first caller")
)]
pub(crate) fn write_change(
    tx: &rusqlite::Transaction<'_>,
    view: &ViewTable,
    project: &ProjectId,
    change: &Change,
    produced_seq: u64,
) -> Result<(), StoreError> {
    let (key, body) = match change {
        Change::Put { key, body } => (key, Some(body)),
        Change::Delete { key } => (key, None),
    };
    let key_values = view.key_values(key)?;
    let generation = live_generation(tx, project)?;
    let mut params = vec![
        SqlValue::Text(project.0.clone()),
        SqlValue::Integer(generation),
    ];
    params.extend(key_values);
    let key_condition: String = view
        .key_columns()
        .map(|column| format!(" AND {column} = ?"))
        .collect();
    let Some(body) = body else {
        tx.execute(
            &format!(
                "DELETE FROM {} WHERE project_id = ? AND generation = ?{key_condition}",
                view.table
            ),
            params_from_iter(params),
        )
        .map_err(sql)?;
        return Ok(());
    };
    for (name, kind) in &view.index_columns {
        let field = body.get(name.as_str());
        let value = match kind {
            FieldKind::Text => field
                .and_then(|value| value.as_str())
                .map(|text| SqlValue::Text(text.to_owned())),
            FieldKind::Integer => field
                .and_then(|value| value.as_i64())
                .map(SqlValue::Integer),
        };
        params.push(value.ok_or_else(|| {
            malformed(
                &view.spec.name,
                format!("the document's {name} is missing or not {kind:?}"),
            )
        })?);
    }
    let produced_seq = i64::try_from(produced_seq)
        .map_err(|_| StoreError::Unavailable(format!("sequence {produced_seq} is past i64")))?;
    params.push(SqlValue::Integer(produced_seq));
    params.push(SqlValue::Integer(i64::from(view.spec.version)));
    params.push(SqlValue::Text(body.to_string()));
    let mut columns = vec!["project_id".to_owned(), "generation".to_owned()];
    columns.extend(view.key_columns());
    columns.extend(
        view.index_columns
            .iter()
            .map(|(name, _)| format!("i_{name}")),
    );
    columns.extend(["produced_seq", "projector_version", "doc_json"].map(String::from));
    let placeholders = vec!["?"; columns.len()].join(", ");
    tx.execute(
        &format!(
            "INSERT OR REPLACE INTO {} ({}) VALUES ({placeholders})",
            view.table,
            columns.join(", ")
        ),
        params_from_iter(params),
    )
    .map_err(sql)?;
    Ok(())
}

/// The text before a cursor's position: the query it belongs to. Every
/// item says its own length, and the equality values are counted, so two
/// different queries never share it.
fn cursor_identity(project: &ProjectId, view: &str, query: &IndexQuery) -> String {
    let mut text = String::from("c1.");
    for part in [&project.0, view, &query.index] {
        encode(&mut text, &KeyValue::Text(part.to_owned()));
    }
    // A vector's length always fits in i64 on the platforms Rust supports.
    encode(&mut text, &KeyValue::Integer(query.equals.len() as i64));
    for value in &query.equals {
        encode(&mut text, value);
    }
    text.push('|');
    text
}

fn encode(text: &mut String, value: &KeyValue) {
    match value {
        KeyValue::Text(value) => {
            text.push_str(&format!("t{}:{value}", value.len()));
        }
        KeyValue::Integer(value) => text.push_str(&format!("i{value};")),
    }
}

fn issue_cursor(identity: &str, position: &[KeyValue]) -> Cursor {
    let mut text = identity.to_owned();
    for value in position {
        encode(&mut text, value);
    }
    Cursor(text)
}

/// The position a cursor holds, or `InvalidCursor` unless this query
/// issued it with exactly the order columns' kinds.
fn read_cursor(
    identity: &str,
    cursor: &Cursor,
    kinds: &[FieldKind],
) -> Result<Vec<KeyValue>, StoreError> {
    let invalid = || StoreError::Refused(Refusal::InvalidCursor);
    let mut rest = cursor.0.strip_prefix(identity).ok_or_else(invalid)?;
    let mut position = Vec::with_capacity(kinds.len());
    for kind in kinds {
        let (value, after) = decode(rest, *kind).ok_or_else(invalid)?;
        position.push(value);
        rest = after;
    }
    if rest.is_empty() {
        Ok(position)
    } else {
        Err(invalid())
    }
}

/// One encoded value of the given kind, and the text after it.
fn decode(text: &str, kind: FieldKind) -> Option<(KeyValue, &str)> {
    match kind {
        FieldKind::Text => {
            let (length, rest) = text.strip_prefix('t')?.split_once(':')?;
            let length: usize = length.parse().ok()?;
            let value = rest.get(..length)?;
            Some((KeyValue::Text(value.to_owned()), &rest[length..]))
        }
        FieldKind::Integer => {
            let (number, rest) = text.strip_prefix('i')?.split_once(';')?;
            Some((KeyValue::Integer(number.parse().ok()?), rest))
        }
    }
}

impl Views for SqliteStore {
    /// The document at `key` in the project's live generation.
    fn get(
        &self,
        project: &ProjectId,
        view: &str,
        key: &DocKey,
    ) -> Result<Option<Document>, StoreError> {
        let table = self.views().table(view)?;
        let key_values = table.key_values(key)?;
        self.snapshot(|conn| {
            let generation = live_generation(conn, project)?;
            let mut params = vec![
                SqlValue::Text(project.0.clone()),
                SqlValue::Integer(generation),
            ];
            params.extend(key_values);
            let key_condition: String = table
                .key_columns()
                .map(|column| format!(" AND {column} = ?"))
                .collect();
            conn.query_row(
                &format!(
                    "SELECT produced_seq, projector_version, doc_json FROM {}
                      WHERE project_id = ? AND generation = ?{key_condition}",
                    table.table
                ),
                params_from_iter(params),
                |row| document(row, key.clone()),
            )
            .optional()
            .map_err(sql)
        })
    }

    /// A page of at most `min(limit, page_bound)` documents by a declared
    /// index, in its order, with a cursor when more follow. A limit of zero
    /// reads nothing and gives no cursor.
    fn find(
        &self,
        project: &ProjectId,
        view: &str,
        query: &IndexQuery,
    ) -> Result<Page<Document>, StoreError> {
        let table = self.views().table(view)?;
        let index = table.index(&query.index)?;
        let equals = table.equals_values(index, &query.equals)?;
        let order = table.order_after(index, query.equals.len());
        let identity = cursor_identity(project, view, query);
        let after = match &query.page.after {
            Some(cursor) => {
                let kinds: Vec<FieldKind> = order.iter().map(|column| column.kind).collect();
                Some(read_cursor(&identity, cursor, &kinds)?)
            }
            None => None,
        };
        let size = query.page.limit.min(table.spec.page_bound) as usize;
        if size == 0 {
            return Ok(Page {
                items: Vec::new(),
                next: None,
            });
        }
        let seeks = table.seeks(index, query.equals.len(), after.as_deref());
        self.snapshot(|conn| {
            let generation = live_generation(conn, project)?;
            // One row past the page says whether another page follows.
            let wanted = size + 1;
            let mut rows: Vec<Found> = Vec::with_capacity(wanted);
            for seek in &seeks {
                let params = seek.params(project, generation, &equals, wanted - rows.len());
                let mut statement = conn.prepare(&seek.sql).map_err(sql)?;
                let found = statement
                    .query_map(params_from_iter(params), |row| table.found(row, &order))
                    .map_err(sql)?;
                for row in found {
                    rows.push(row.map_err(sql)?);
                }
                if rows.len() == wanted {
                    break;
                }
            }
            let next = if rows.len() > size {
                rows.truncate(size);
                rows.last()
                    .map(|last| issue_cursor(&identity, &last.position))
            } else {
                None
            };
            Ok(Page {
                items: rows.into_iter().map(|row| row.document).collect(),
                next,
            })
        })
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use baley_store::{FieldSpec, IndexField, PageRequest};

    use super::*;
    use crate::store::Options;

    const AT: &str = "2026-09-25T18:00:00Z";

    fn index_field(name: &str, kind: FieldKind, order: Order) -> IndexField {
        IndexField {
            name: name.into(),
            kind,
            order,
        }
    }

    /// Items keyed by id, with one index of mixed orders and one plain.
    fn item_view() -> ViewSpec {
        ViewSpec {
            name: "item".into(),
            version: 3,
            key: vec![FieldSpec {
                name: "id".into(),
                kind: FieldKind::Integer,
            }],
            indexes: vec![
                IndexSpec {
                    name: "by_state_rank".into(),
                    fields: vec![
                        index_field("state", FieldKind::Text, Order::Ascending),
                        index_field("rank", FieldKind::Integer, Order::Descending),
                    ],
                },
                IndexSpec {
                    name: "by_owner".into(),
                    fields: vec![index_field("owner", FieldKind::Text, Order::Ascending)],
                },
            ],
            page_bound: 3,
        }
    }

    fn project() -> ProjectId {
        ProjectId("p1".into())
    }

    /// A store declaring `item`, holding the project `p1`.
    fn open(home: &Path) -> SqliteStore {
        let options = Options {
            views: vec![item_view()],
            ..Options::default()
        };
        let store = SqliteStore::open(home, AT, options).expect("open");
        store
            .write(|tx| {
                tx.execute(
                    "INSERT INTO project (project_id, name, created_at) VALUES ('p1', 'one', ?1)",
                    [AT],
                )
                .map_err(sql)?;
                Ok(())
            })
            .expect("project");
        store
    }

    fn key(id: i64) -> DocKey {
        DocKey(vec![KeyValue::Integer(id)])
    }

    fn item_json(id: i64, state: &str, rank: i64, owner: &str) -> String {
        format!(r#"{{"id": {id}, "state": "{state}", "rank": {rank}, "owner": "{owner}"}}"#)
    }

    fn apply(store: &SqliteStore, change: &Change, seq: u64) -> Result<(), StoreError> {
        store.write(|tx| write_change(tx, store.views().table("item")?, &project(), change, seq))
    }

    fn put(store: &SqliteStore, id: i64, state: &str, rank: i64, owner: &str, seq: u64) {
        let change = Change::Put {
            key: key(id),
            body: item_json(id, state, rank, owner).parse().expect("json"),
        };
        apply(store, &change, seq).expect("put");
    }

    /// Seven items with ties on (state, rank), so only the key orders them.
    /// By `by_state_rank` (state ascending, rank descending, id ascending)
    /// they read 4, 6, 2, 7, 1, 3, 5.
    fn stocked(home: &Path) -> SqliteStore {
        let store = open(home);
        for (id, state, rank, owner, seq) in [
            (1, "open", 5, "ann", 11),
            (2, "open", 7, "bob", 12),
            (3, "open", 5, "cat", 13),
            (4, "done", 9, "ann", 14),
            (5, "open", 5, "ann", 15),
            (6, "done", 2, "bob", 16),
            (7, "open", 7, "ann", 17),
        ] {
            put(&store, id, state, rank, owner, seq);
        }
        store
    }

    fn query(index: &str, equals: &[KeyValue], limit: u32, after: Option<Cursor>) -> IndexQuery {
        IndexQuery {
            index: index.into(),
            equals: equals.to_vec(),
            page: PageRequest { limit, after },
        }
    }

    /// Every page from the first to the last, as ids.
    fn pages(store: &SqliteStore, index: &str, equals: &[KeyValue], limit: u32) -> Vec<Vec<i64>> {
        let mut pages = Vec::new();
        let mut after = None;
        loop {
            let page = store
                .find(&project(), "item", &query(index, equals, limit, after))
                .expect("find");
            pages.push(
                page.items
                    .iter()
                    .map(|document| match document.key.0.as_slice() {
                        [KeyValue::Integer(id)] => *id,
                        other => panic!("not an item key: {other:?}"),
                    })
                    .collect(),
            );
            assert!(pages.len() <= 10, "paging does not end: {pages:?}");
            match page.next {
                Some(cursor) => after = Some(cursor),
                None => return pages,
            }
        }
    }

    fn text(value: &str) -> KeyValue {
        KeyValue::Text(value.into())
    }

    // Pages of two follow state ascending, rank descending, then id, and
    // each cursor picks up exactly where its page ended, across the ties on
    // rank 5 and 7. Catches a cursor keyed on the non-unique index fields,
    // which skips or repeats tied items, and a descending field paged as
    // ascending.
    #[test]
    fn pages_follow_the_declared_order_and_continue_without_gaps_or_repeats() {
        let home = tempfile::tempdir().expect("temp dir");
        let store = stocked(home.path());
        assert_eq!(
            pages(&store, "by_state_rank", &[], 2),
            [vec![4, 6], vec![2, 7], vec![1, 3], vec![5]]
        );
    }

    // Asking for ten from a view bounded at three gets pages of three.
    // Catches a limit that ignores the view's bound.
    #[test]
    fn no_page_holds_more_than_the_views_bound() {
        let home = tempfile::tempdir().expect("temp dir");
        let store = stocked(home.path());
        assert_eq!(
            pages(&store, "by_state_rank", &[], 10),
            [vec![4, 6, 2], vec![7, 1, 3], vec![5]]
        );
    }

    // Equality on a leading prefix keeps only matching items, still in the
    // index's order for the rest. Catches equality values bound to the
    // wrong columns, and a prefix that drops the remaining order.
    #[test]
    fn equality_on_a_prefix_selects_and_keeps_the_rest_of_the_order() {
        let home = tempfile::tempdir().expect("temp dir");
        let store = stocked(home.path());
        assert_eq!(
            pages(&store, "by_state_rank", &[text("open")], 2),
            [vec![2, 7], vec![1, 3], vec![5]]
        );
        assert_eq!(
            pages(
                &store,
                "by_state_rank",
                &[text("open"), KeyValue::Integer(5)],
                10
            ),
            [vec![1, 3, 5]]
        );
        assert_eq!(
            pages(&store, "by_owner", &[text("ann")], 10),
            [vec![1, 4, 5], vec![7]]
        );
    }

    /// A position of the right kinds for the order after `fixed` fields.
    fn some_position(table: &ViewTable, index: &IndexSpec, fixed: usize) -> Vec<KeyValue> {
        table
            .order_after(index, fixed)
            .iter()
            .map(|column| match column.kind {
                FieldKind::Text => text("m"),
                FieldKind::Integer => KeyValue::Integer(4),
            })
            .collect()
    }

    fn some_equals(index: &IndexSpec, fixed: usize) -> Vec<SqlValue> {
        index.fields[..fixed]
            .iter()
            .map(|field| match field.kind {
                FieldKind::Text => SqlValue::Text("open".into()),
                FieldKind::Integer => SqlValue::Integer(5),
            })
            .collect()
    }

    // Every statement `find` can run, on each declared index, from the
    // start and after a cursor, with each length of equality prefix, is
    // planned by SQLite as one search of that index, with no scan and no
    // temporary sort. Catches a page read by scanning, or sorted in memory
    // after reading, and a continuation that filters instead of seeking.
    #[test]
    fn every_find_statement_searches_its_index_without_a_scan_or_sort() {
        let home = tempfile::tempdir().expect("temp dir");
        let store = stocked(home.path());
        let table = store.views().table("item").expect("item");
        let mut checked = 0;
        for index in &table.spec.indexes {
            let expected = format!("SEARCH v_item USING INDEX v_item__{} (", index.name);
            for fixed in 0..=index.fields.len() {
                let position = some_position(table, index, fixed);
                for after in [None, Some(position.as_slice())] {
                    for seek in table.seeks(index, fixed, after) {
                        let params = seek.params(&project(), 0, &some_equals(index, fixed), 4);
                        let plan: Vec<String> = store
                            .snapshot(|conn| {
                                let mut statement = conn
                                    .prepare(&format!("EXPLAIN QUERY PLAN {}", seek.sql))
                                    .map_err(sql)?;
                                statement
                                    .query_map(params_from_iter(params), |row| row.get(3))
                                    .map_err(sql)?
                                    .collect::<rusqlite::Result<_>>()
                                    .map_err(sql)
                            })
                            .expect("plan");
                        assert!(
                            plan.len() == 1 && plan[0].starts_with(&expected),
                            "{}: {plan:?}",
                            seek.sql
                        );
                        checked += 1;
                    }
                }
            }
        }
        // by_state_rank: 1 + 3, 1 + 2, 1 + 1; by_owner: 1 + 2, 1 + 1.
        assert_eq!(checked, 14);
    }

    // Each statement stops at the limit it is given, inside SQLite: run on
    // its own over seven items with a limit of two, none returns more.
    // Catches a limit applied in Rust after every matching row was read.
    #[test]
    fn each_find_statement_is_limited_in_sql() {
        let home = tempfile::tempdir().expect("temp dir");
        let store = stocked(home.path());
        let table = store.views().table("item").expect("item");
        let index = table.index("by_state_rank").expect("index");
        // After item 4, (done, 9, 4).
        let after = [text("done"), KeyValue::Integer(9), KeyValue::Integer(4)];
        let mut counts = Vec::new();
        for seek in
            table
                .seeks(index, 0, None)
                .into_iter()
                .chain(table.seeks(index, 0, Some(&after)))
        {
            let params = seek.params(&project(), 0, &[], 2);
            let rows: i64 = store
                .snapshot(|conn| {
                    conn.query_row(
                        &format!("SELECT count(*) FROM ({})", seek.sql),
                        params_from_iter(params),
                        |row| row.get(0),
                    )
                    .map_err(sql)
                })
                .expect("count");
            counts.push(rows);
        }
        // From the start: seven match. After item 4: past id 4 at
        // (done, 9), none; past rank 9 in done, item 6; past done, the
        // five open items.
        assert_eq!(counts, [2, 0, 1, 2]);
    }

    // Catches `find` falling back to some other index or a scan when the
    // named index is not declared.
    #[test]
    fn an_undeclared_index_is_refused() {
        let home = tempfile::tempdir().expect("temp dir");
        let store = stocked(home.path());
        assert_eq!(
            store.find(&project(), "item", &query("by_rank", &[], 2, None)),
            Err(StoreError::Refused(Refusal::UndeclaredIndex {
                view: "item".into(),
                index: "by_rank".into(),
            }))
        );
    }

    // Catches a read of an undeclared view reaching SQL, here a table of
    // the ledger's own.
    #[test]
    fn an_unknown_view_is_refused() {
        let home = tempfile::tempdir().expect("temp dir");
        let store = stocked(home.path());
        assert_eq!(
            store.get(&project(), "event", &key(1)),
            Err(StoreError::Refused(Refusal::UnknownView("event".into())))
        );
    }

    // A key of the wrong kind or length is refused, not matched loosely.
    // Catches a text key compared with an integer column, and extra or
    // missing key values ignored.
    #[test]
    fn a_key_that_does_not_fit_the_declared_fields_is_refused() {
        let home = tempfile::tempdir().expect("temp dir");
        let store = stocked(home.path());
        for bad in [
            DocKey(vec![text("1")]),
            DocKey(vec![KeyValue::Integer(1), KeyValue::Integer(2)]),
            DocKey(Vec::new()),
        ] {
            assert!(
                matches!(
                    store.get(&project(), "item", &bad),
                    Err(StoreError::Refused(Refusal::MalformedKey { .. }))
                ),
                "{bad:?}"
            );
        }
    }

    // Equality values of the wrong kind, or more than the index has
    // fields, are refused. Catches values bound to columns they do not fit.
    #[test]
    fn equality_values_that_do_not_fit_the_index_are_refused() {
        let home = tempfile::tempdir().expect("temp dir");
        let store = stocked(home.path());
        for equals in [
            vec![KeyValue::Integer(1)],
            vec![text("open"), text("5")],
            vec![text("open"), KeyValue::Integer(5), KeyValue::Integer(1)],
        ] {
            assert!(
                matches!(
                    store.find(
                        &project(),
                        "item",
                        &query("by_state_rank", &equals, 2, None)
                    ),
                    Err(StoreError::Refused(Refusal::MalformedKey { .. }))
                ),
                "{equals:?}"
            );
        }
    }

    // A cursor is refused by any other query: other equality values,
    // another index, another project, or altered text. Catches a cursor
    // read as a bare position, which would page one query from another's
    // place.
    #[test]
    fn a_cursor_this_query_did_not_issue_is_refused() {
        let home = tempfile::tempdir().expect("temp dir");
        let store = stocked(home.path());
        let open_items = [text("open")];
        let cursor = store
            .find(
                &project(),
                "item",
                &query("by_state_rank", &open_items, 2, None),
            )
            .expect("find")
            .next
            .expect("a next page");
        let mut altered = cursor.clone();
        altered.0.pop();
        let other_project = ProjectId("p2".into());
        let refused = Err(StoreError::Refused(Refusal::InvalidCursor));
        for (in_project, other) in [
            (
                project(),
                query("by_state_rank", &[text("done")], 2, Some(cursor.clone())),
            ),
            (
                project(),
                query("by_owner", &open_items, 2, Some(cursor.clone())),
            ),
            (
                project(),
                query("by_state_rank", &open_items, 2, Some(altered)),
            ),
            (
                other_project,
                query("by_state_rank", &open_items, 2, Some(cursor.clone())),
            ),
        ] {
            assert_eq!(
                store.find(&in_project, "item", &other),
                refused,
                "{other:?}"
            );
        }
    }

    // `get` returns the document last put at the key, with the sequence
    // that put it and the view's projector version. Catches a put that
    // inserts beside the old row, and provenance not stored or not read.
    #[test]
    fn get_returns_the_last_put_with_its_provenance() {
        let home = tempfile::tempdir().expect("temp dir");
        let store = open(home.path());
        put(&store, 1, "open", 5, "ann", 11);
        put(&store, 1, "done", 8, "ann", 20);
        let expected = Document {
            key: key(1),
            produced_seq: 20,
            projector_version: 3,
            body: item_json(1, "done", 8, "ann").parse().expect("json"),
        };
        assert_eq!(store.get(&project(), "item", &key(1)), Ok(Some(expected)));
    }

    // Catches a delete that misses the row, or removes the wrong one.
    #[test]
    fn a_deleted_document_reads_absent() {
        let home = tempfile::tempdir().expect("temp dir");
        let store = open(home.path());
        put(&store, 1, "open", 5, "ann", 11);
        put(&store, 2, "open", 5, "ann", 12);
        apply(&store, &Change::Delete { key: key(1) }, 13).expect("delete");
        assert_eq!(store.get(&project(), "item", &key(1)), Ok(None));
        assert!(matches!(
            store.get(&project(), "item", &key(2)),
            Ok(Some(_))
        ));
    }

    // Once `project_gen` names generation 1 live, a document written in
    // generation 0 is no longer read, and new writes land in generation 1.
    // Catches a generation fixed at 0 in the reads or the writes.
    #[test]
    fn reads_and_writes_follow_the_live_generation() {
        let home = tempfile::tempdir().expect("temp dir");
        let store = open(home.path());
        put(&store, 1, "open", 5, "ann", 11);
        store
            .write(|tx| {
                tx.execute(
                    "INSERT INTO project_gen (project_id, live_gen) VALUES ('p1', 1)",
                    [],
                )
                .map_err(sql)?;
                Ok(())
            })
            .expect("flip");
        put(&store, 2, "open", 5, "ann", 12);
        assert_eq!(store.get(&project(), "item", &key(1)), Ok(None));
        assert!(matches!(
            store.get(&project(), "item", &key(2)),
            Ok(Some(_))
        ));
        let generations: Vec<(i64, i64)> = store
            .read(|conn| {
                let mut statement =
                    conn.prepare("SELECT k_id, generation FROM v_item ORDER BY k_id")?;
                statement
                    .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?
                    .collect()
            })
            .expect("rows");
        assert_eq!(generations, [(1, 0), (2, 1)]);
    }

    // Catches a missing project read as an empty view.
    #[test]
    fn a_read_in_an_unknown_project_is_refused() {
        let home = tempfile::tempdir().expect("temp dir");
        let store = open(home.path());
        let other = ProjectId("p2".into());
        assert_eq!(
            store.get(&other, "item", &key(1)),
            Err(StoreError::Refused(Refusal::UnknownProject(other)))
        );
    }

    // A body missing an index field, or holding it as the wrong kind, is
    // refused and writes nothing. Catches a null or mistyped index value,
    // which would fall outside the index's total order.
    #[test]
    fn a_document_without_its_index_fields_is_refused() {
        let home = tempfile::tempdir().expect("temp dir");
        let store = open(home.path());
        for body in [
            r#"{"id": 1, "rank": 5, "owner": "ann"}"#,
            r#"{"id": 1, "state": "open", "rank": "5", "owner": "ann"}"#,
        ] {
            let change = Change::Put {
                key: key(1),
                body: body.parse().expect("json"),
            };
            assert!(
                matches!(
                    apply(&store, &change, 11),
                    Err(StoreError::Refused(Refusal::MalformedKey { .. }))
                ),
                "{body}"
            );
        }
        assert_eq!(store.get(&project(), "item", &key(1)), Ok(None));
    }

    // Names become SQL identifiers unquoted, so only plain lowercase runs
    // joined by single underscores pass, in every place a name appears.
    // Catches SQL text reaching a statement through a name, and two
    // (view, index) pairs sharing an index name through a double
    // underscore.
    #[test]
    fn names_that_are_not_safe_identifiers_are_refused() {
        let with_view_name = |name: &str| ViewSpec {
            name: name.into(),
            ..item_view()
        };
        assert!(ViewTable::new(&with_view_name("phase_2")).is_ok());
        let too_long = "a".repeat(MAX_NAME + 1);
        for name in [
            "",
            "Item",
            "item; DROP TABLE event",
            "a__b",
            "_a",
            "a_",
            "2a",
            "é",
            too_long.as_str(),
        ] {
            assert!(ViewTable::new(&with_view_name(name)).is_err(), "{name:?}");
        }
        let mut bad_key = item_view();
        bad_key.key[0].name = "id)".into();
        let mut bad_index = item_view();
        bad_index.indexes[0].name = "by state".into();
        let mut bad_field = item_view();
        bad_field.indexes[0].fields[0].name = "state DESC".into();
        for spec in [bad_key, bad_index, bad_field] {
            assert!(ViewTable::new(&spec).is_err(), "{spec:?}");
        }
    }

    // A store a newer binary stamped opens read-only and gets no view
    // tables. Catches view creation that skips the epoch rule.
    #[test]
    fn a_read_only_store_creates_no_view_table() {
        let home = tempfile::tempdir().expect("temp dir");
        drop(SqliteStore::open(home.path(), AT, Options::default()).expect("open"));
        let raw = Connection::open(home.path().join("baley.db")).expect("raw");
        raw.execute("UPDATE schema_meta SET value = 2 WHERE key = 'epoch'", [])
            .expect("stamp");
        let options = Options {
            views: vec![item_view()],
            ..Options::default()
        };
        drop(SqliteStore::open(home.path(), AT, options).expect("open read-only"));
        let views: i64 = raw
            .query_row(
                "SELECT count(*) FROM sqlite_schema WHERE name LIKE 'v\\_%' ESCAPE '\\'",
                [],
                |row| row.get(0),
            )
            .expect("count");
        assert_eq!(views, 0);
    }
}
