//! Views in the SQLite adapter (design 0001, The storage port; Views and
//! projectors; Physical schema; EVD-R9).
//!
//! Each declared view version is one table, `v_<view>_<version>`, with the
//! project, the generation, one column per key field (`k_<field>`), one per
//! index field (`i_<field>`), the provenance and the document. Key and index
//! columns copy the document body's top-level fields. Each declared index is
//! a SQL index, `v_<view>_<version>__<index>`, on the project, the
//! generation, its fields in their orders, then the key, so every order is
//! total. `view_catalog` keeps each version's spec as text, so a spec changed
//! without a new version is refused instead of read through the old table.
//! `find` pages through its index with seeks bounded in SQL, never a scan or
//! a sort.

use std::collections::{BTreeMap, BTreeSet};

use baley_store::{
    Change, Cursor, DocKey, Document, FieldKind, IndexQuery, IndexSpec, KeyValue, Order, Page,
    ProjectId, Refusal, StoreError, ViewSpec, Views, canonical_json,
};
use rusqlite::types::Value as SqlValue;
use rusqlite::{Connection, OptionalExtension, Row, params, params_from_iter};
use serde_json::Value;

use crate::store::{SqliteStore, sql};

/// The longest view, index or field name accepted. The longest physical
/// name, an index's, is then 2 + 48 + 1 + 10 + 2 + 48 characters; SQLite
/// sets no limit of its own on identifiers.
const MAX_NAME: usize = 48;

/// The declared views, checked once when the store opens.
pub(crate) struct ViewSet(BTreeMap<String, ViewTable>);

/// One checked view and the SQL names it becomes.
pub(crate) struct ViewTable {
    spec: ViewSpec,
    /// `v_<view>_<version>`. The version is always the last run of digits,
    /// so no two (view, version) pairs share a table.
    table: String,
    /// The spec as `view_catalog` stores it.
    rendered: String,
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

    /// Whether no view is declared, so open has nothing to check.
    pub(crate) fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Whether any view still needs its catalog row, table or an index.
    /// Refuses a version stored with another spec. Reads only, so open can
    /// ask on the read connection before it takes the writer queue.
    pub(crate) fn pending(&self, conn: &Connection) -> Result<bool, StoreError> {
        let mut pending = false;
        for table in self.0.values() {
            pending |= table.pending(conn)?;
        }
        Ok(pending)
    }

    /// Checks again inside the write transaction, then creates whatever is
    /// still missing.
    pub(crate) fn create(&self, tx: &rusqlite::Transaction<'_>) -> Result<(), StoreError> {
        for table in self.0.values() {
            if !table.pending(tx)? {
                continue;
            }
            tx.execute_batch(&table.create_sql()).map_err(sql)?;
            tx.execute(
                "INSERT OR IGNORE INTO view_catalog (view, version, spec) VALUES (?1, ?2, ?3)",
                params![table.spec.name, table.spec.version, table.rendered],
            )
            .map_err(sql)?;
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
            table: format!("v_{}_{}", spec.name, spec.version),
            rendered: render(spec),
            index_columns,
        })
    }

    /// Whether this version's catalog row, table or an index is missing.
    /// A catalog row with another spec is refused: the spec changed without
    /// a new version, and the stored table no longer fits it.
    fn pending(&self, conn: &Connection) -> Result<bool, StoreError> {
        let stored: Option<String> = conn
            .query_row(
                "SELECT spec FROM view_catalog WHERE view = ?1 AND version = ?2",
                params![self.spec.name, self.spec.version],
                |row| row.get(0),
            )
            .optional()
            .map_err(sql)?;
        match stored {
            None => return Ok(true),
            Some(stored) if stored != self.rendered => {
                return Err(malformed(
                    &self.spec.name,
                    format!(
                        "version {} is stored with another spec; a changed spec needs a new version",
                        self.spec.version
                    ),
                ));
            }
            Some(_) => {}
        }
        let mut names = vec![self.table.clone()];
        names.extend(self.spec.indexes.iter().map(|index| self.index_name(index)));
        let placeholders = vec!["?"; names.len()].join(", ");
        let present: i64 = conn
            .query_row(
                &format!("SELECT count(*) FROM sqlite_schema WHERE name IN ({placeholders})"),
                params_from_iter(&names),
                |row| row.get(0),
            )
            .map_err(sql)?;
        Ok(usize::try_from(present) != Ok(names.len()))
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

    /// `v_<view>_<version>__<index>`. Names hold no double underscore, so
    /// the first one ends the table's part and no two indexes share a name.
    fn index_name(&self, index: &IndexSpec) -> String {
        format!("{}__{}", self.table, index.name)
    }

    fn key_columns(&self) -> impl Iterator<Item = String> + '_ {
        self.spec
            .key
            .iter()
            .map(|field| format!("k_{}", field.name))
    }

    fn key_condition(&self) -> String {
        self.key_columns()
            .map(|column| format!(" AND {column} = ?"))
            .collect()
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

    /// A staged document's position in a page's order, or `None` when it
    /// does not match the query's equality values. A body without a
    /// declared index field is refused, as its write would be.
    fn staged_position(
        &self,
        index: &IndexSpec,
        equals: &[KeyValue],
        order: &[OrderColumn],
        document: &Document,
    ) -> Result<Option<Vec<KeyValue>>, StoreError> {
        let field = |name: &str, kind: FieldKind| {
            let held = document.body.get(name);
            let value = match kind {
                FieldKind::Text => held
                    .and_then(|value| value.as_str())
                    .map(|text| KeyValue::Text(text.to_owned())),
                FieldKind::Integer => held.and_then(|value| value.as_i64()).map(KeyValue::Integer),
            };
            value.ok_or_else(|| {
                malformed(
                    &self.spec.name,
                    format!("the document's {name} is missing or not {kind:?}"),
                )
            })
        };
        for (spec, wanted) in index.fields.iter().zip(equals) {
            if field(&spec.name, spec.kind)? != *wanted {
                return Ok(None);
            }
        }
        let mut position = Vec::with_capacity(order.len());
        let mut key = document.key.0.iter();
        for column in order {
            match column.column.strip_prefix("i_") {
                Some(name) => position.push(field(name, column.kind)?),
                None => position.push(key.next().cloned().ok_or_else(|| {
                    malformed(&self.spec.name, "the key is shorter than declared".into())
                })?),
            }
        }
        Ok(Some(position))
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

/// The spec as `view_catalog` keeps it: one line per part, every field with
/// its kind and, in an index, its order. Names are plain identifiers, so
/// the text needs no escaping and one spec has exactly one rendering.
fn render(spec: &ViewSpec) -> String {
    let mut text = format!("view {} version {}\nkey", spec.name, spec.version);
    for field in &spec.key {
        text.push_str(&format!(" {} {}", field.name, kind_name(field.kind)));
    }
    for index in &spec.indexes {
        text.push_str(&format!("\nindex {}", index.name));
        for field in &index.fields {
            text.push_str(&format!(
                " {} {} {}",
                field.name,
                kind_name(field.kind),
                direction(field.order)
            ));
        }
    }
    text.push_str(&format!("\npage_bound {}\n", spec.page_bound));
    text
}

/// The rule for every declared name: a lowercase ASCII letter, then
/// lowercase letters and digits in runs joined by single underscores.
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

fn kind_name(kind: FieldKind) -> &'static str {
    match kind {
        FieldKind::Text => "text",
        FieldKind::Integer => "integer",
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

/// Puts or deletes one document in `generation`, stamped with the event
/// that produced it and the view's projector version. A put's body must
/// hold every key field equal to `key` and every index field, each of its
/// declared kind; otherwise nothing is written, so the columns always agree
/// with the body and every index order stays total.
pub(crate) fn write_change(
    tx: &rusqlite::Transaction<'_>,
    view: &ViewTable,
    project: &ProjectId,
    generation: i64,
    change: &Change,
    produced_seq: u64,
) -> Result<(), StoreError> {
    let (key, body) = match change {
        Change::Put { key, body } => (key, Some(body)),
        Change::Delete { key } => (key, None),
    };
    let key_values = view.key_values(key)?;
    let mut params = vec![
        SqlValue::Text(project.0.clone()),
        SqlValue::Integer(generation),
    ];
    params.extend(key_values);
    let Some(body) = body else {
        tx.execute(
            &format!(
                "DELETE FROM {} WHERE project_id = ? AND generation = ?{}",
                view.table,
                view.key_condition()
            ),
            params_from_iter(params),
        )
        .map_err(sql)?;
        return Ok(());
    };
    for (field, value) in view.spec.key.iter().zip(&key.0) {
        let held = body.get(field.name.as_str());
        let equal = match value {
            KeyValue::Text(text) => held.and_then(|held| held.as_str()) == Some(text.as_str()),
            KeyValue::Integer(number) => held.and_then(|held| held.as_i64()) == Some(*number),
        };
        if !equal {
            return Err(malformed(
                &view.spec.name,
                format!("the document's {} is missing or not its key", field.name),
            ));
        }
    }
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
    // Canonical text, so the stored bytes do not depend on how serde_json
    // was built.
    let text = canonical_json(body)
        .map_err(|error| malformed(&view.spec.name, format!("the document: {error}")))?;
    params.push(SqlValue::Text(String::from_utf8(text).map_err(|_| {
        StoreError::Unavailable("canonical JSON that is not UTF-8".into())
    })?));
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

/// `write_change` into the project's live generation, for ordinary writes.
pub(crate) fn write_live(
    tx: &rusqlite::Transaction<'_>,
    view: &ViewTable,
    project: &ProjectId,
    change: &Change,
    produced_seq: u64,
) -> Result<(), StoreError> {
    let generation = live_generation(tx, project)?;
    write_change(tx, view, project, generation, change, produced_seq)
}

/// The document at `key` in the project's live generation. Takes any
/// connection, so a write transaction reads its own writes.
pub(crate) fn get_document(
    conn: &Connection,
    view: &ViewTable,
    project: &ProjectId,
    key: &DocKey,
) -> Result<Option<Document>, StoreError> {
    let key_values = view.key_values(key)?;
    let generation = live_generation(conn, project)?;
    let mut params = vec![
        SqlValue::Text(project.0.clone()),
        SqlValue::Integer(generation),
    ];
    params.extend(key_values);
    conn.query_row(
        &format!(
            "SELECT produced_seq, projector_version, doc_json FROM {}
              WHERE project_id = ? AND generation = ?{}",
            view.table,
            view.key_condition()
        ),
        params_from_iter(params),
        |row| document(row, key.clone()),
    )
    .optional()
    .map_err(sql)
}

/// A document the write transaction has changed and not yet written: its
/// body, or `None` once deleted, and the event that last changed it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Staged {
    pub(crate) body: Option<Value>,
    pub(crate) produced_seq: u64,
}

impl Staged {
    /// The document a read sees, or `None` for a deletion.
    pub(crate) fn document(&self, view: &ViewTable, key: &DocKey) -> Option<Document> {
        self.body.as_ref().map(|body| Document {
            key: key.clone(),
            produced_seq: self.produced_seq,
            projector_version: view.spec.version,
            body: body.clone(),
        })
    }
}

/// A page of at most `min(limit, page_bound)` documents by a declared
/// index, in its order, with a cursor when more follow. A cursor issued
/// under another generation or view version is refused. A limit of zero
/// reads nothing and gives no cursor. Takes any connection, so a write
/// transaction reads what it has written.
pub(crate) fn find_documents(
    conn: &Connection,
    view: &ViewTable,
    project: &ProjectId,
    query: &IndexQuery,
) -> Result<Page<Document>, StoreError> {
    find_with_staged(conn, view, project, query, &BTreeMap::new())
}

/// `find_documents` over the stored documents with the transaction's
/// `staged` changes to this view laid on top, so a decision's `find` sees
/// what its own events changed before any of it is written. Each staged key
/// hides its stored row; each staged body that matches the query takes its
/// place in the index order. The stored rows are read `staged.len()` past
/// the page, which covers every row a staged key can hide.
pub(crate) fn find_with_staged(
    conn: &Connection,
    view: &ViewTable,
    project: &ProjectId,
    query: &IndexQuery,
    staged: &BTreeMap<DocKey, Staged>,
) -> Result<Page<Document>, StoreError> {
    let index = view.index(&query.index)?;
    let equals = view.equals_values(index, &query.equals)?;
    let fixed = query.equals.len();
    let order = view.order_after(index, fixed);
    let generation = live_generation(conn, project)?;
    let identity = cursor_identity(project, view, generation, query);
    let after = match &query.page.after {
        Some(cursor) => {
            let kinds: Vec<FieldKind> = order.iter().map(|column| column.kind).collect();
            Some(read_cursor(&identity, cursor, &kinds)?)
        }
        None => None,
    };
    let size = query.page.limit.min(view.spec.page_bound) as usize;
    if size == 0 {
        return Ok(Page {
            items: Vec::new(),
            next: None,
        });
    }
    // One row past the page says whether another page follows.
    let wanted = size + 1;
    let stored_wanted = wanted + staged.len();
    let mut rows: Vec<Found> = Vec::with_capacity(stored_wanted);
    for seek in view.seeks(index, fixed, after.as_deref()) {
        let params = seek.params(project, generation, &equals, stored_wanted - rows.len());
        let mut statement = conn.prepare(&seek.sql).map_err(sql)?;
        let found = statement
            .query_map(params_from_iter(params), |row| view.found(row, &order))
            .map_err(sql)?;
        for row in found {
            rows.push(row.map_err(sql)?);
        }
        if rows.len() == stored_wanted {
            break;
        }
    }
    if !staged.is_empty() {
        rows.retain(|row| !staged.contains_key(&row.document.key));
        for (key, change) in staged {
            let Some(document) = change.document(view, key) else {
                continue;
            };
            let Some(position) = view.staged_position(index, &query.equals, &order, &document)?
            else {
                continue;
            };
            if after
                .as_deref()
                .is_some_and(|after| compare(&position, after, &order).is_le())
            {
                continue;
            }
            rows.push(Found { document, position });
        }
        rows.sort_by(|left, right| compare(&left.position, &right.position, &order));
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
}

/// Two positions in a page's order: column by column, each in its own
/// direction. Text compares by UTF-8 bytes, as SQLite's binary collation
/// does.
fn compare(left: &[KeyValue], right: &[KeyValue], order: &[OrderColumn]) -> std::cmp::Ordering {
    for ((left, right), column) in left.iter().zip(right).zip(order) {
        let ordering = match column.order {
            Order::Ascending => left.cmp(right),
            Order::Descending => right.cmp(left),
        };
        if ordering.is_ne() {
            return ordering;
        }
    }
    std::cmp::Ordering::Equal
}

/// The text before a cursor's position: the query it belongs to, and the
/// view version and generation it was read under. Every item says its own
/// length, and the equality values are counted, so two different queries
/// never share it.
fn cursor_identity(
    project: &ProjectId,
    view: &ViewTable,
    generation: i64,
    query: &IndexQuery,
) -> String {
    let mut text = String::from("c1.");
    encode(&mut text, &KeyValue::Text(project.0.clone()));
    encode(&mut text, &KeyValue::Text(view.spec.name.clone()));
    encode(&mut text, &KeyValue::Integer(i64::from(view.spec.version)));
    encode(&mut text, &KeyValue::Integer(generation));
    encode(&mut text, &KeyValue::Text(query.index.clone()));
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
        self.snapshot(|conn| get_document(conn, table, project, key))
    }

    /// A page by a declared index, read in one snapshot; see
    /// `find_documents`.
    fn find(
        &self,
        project: &ProjectId,
        view: &str,
        query: &IndexQuery,
    ) -> Result<Page<Document>, StoreError> {
        let table = self.views().table(view)?;
        self.snapshot(|conn| find_documents(conn, table, project, query))
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use baley_store::{FieldSpec, IndexField, PageRequest, Projector};

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

    /// A projector that declares a view and handles no event, so these
    /// tests write documents directly.
    struct Declares(ViewSpec);

    impl Projector for Declares {
        fn spec(&self) -> &ViewSpec {
            &self.0
        }

        fn handles(&self) -> &[&str] {
            &[]
        }

        fn keys(&self, _event: &baley_store::Event) -> Vec<DocKey> {
            Vec::new()
        }

        fn apply(
            &self,
            _event: &baley_store::Event,
            _documents: &[(DocKey, Value)],
        ) -> Result<Vec<Change>, baley_store::ProjectorError> {
            Ok(Vec::new())
        }
    }

    fn open_with(home: &Path, views: Vec<ViewSpec>) -> Result<SqliteStore, StoreError> {
        let options = Options {
            projectors: views
                .into_iter()
                .map(|spec| Box::new(Declares(spec)) as Box<dyn Projector>)
                .collect(),
            ..Options::default()
        };
        SqliteStore::open(home, AT, options)
    }

    /// A store declaring `item`, holding the project `p1`.
    fn open(home: &Path) -> SqliteStore {
        let store = open_with(home, vec![item_view()]).expect("open");
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

    /// A connection of the test's own, beside the store's.
    fn raw(home: &Path) -> Connection {
        Connection::open(home.join("baley.db")).expect("raw connection")
    }

    fn key(id: i64) -> DocKey {
        DocKey(vec![KeyValue::Integer(id)])
    }

    fn item_json(id: i64, state: &str, rank: i64, owner: &str) -> String {
        format!(r#"{{"id": {id}, "state": "{state}", "rank": {rank}, "owner": "{owner}"}}"#)
    }

    fn item(id: i64, state: &str, rank: i64, owner: &str) -> Change {
        Change::Put {
            key: key(id),
            body: item_json(id, state, rank, owner).parse().expect("json"),
        }
    }

    fn apply(store: &SqliteStore, change: &Change, seq: u64) -> Result<(), StoreError> {
        store.write(|tx| write_live(tx, store.views().table("item")?, &project(), change, seq))
    }

    fn put(store: &SqliteStore, id: i64, state: &str, rank: i64, owner: &str, seq: u64) {
        apply(store, &item(id, state, rank, owner), seq).expect("put");
    }

    fn flip_to_generation_1(store: &SqliteStore) {
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

    /// The declared views' tables and indexes. Every store also holds its
    /// own `request` view, which these tests leave out.
    fn view_names(conn: &Connection) -> Vec<String> {
        let mut statement = conn
            .prepare(
                "SELECT name FROM sqlite_schema WHERE name LIKE 'v\\_%' ESCAPE '\\'
                   AND name NOT LIKE 'v\\_request\\_%' ESCAPE '\\' ORDER BY name",
            )
            .expect("prepare");
        statement
            .query_map([], |row| row.get(0))
            .expect("names")
            .collect::<rusqlite::Result<_>>()
            .expect("names")
    }

    fn catalog(conn: &Connection) -> Vec<(String, i64)> {
        let mut statement = conn
            .prepare(
                "SELECT view, version FROM view_catalog WHERE view <> 'request'
                  ORDER BY view, version",
            )
            .expect("prepare");
        statement
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
            .expect("catalog")
            .collect::<rusqlite::Result<_>>()
            .expect("catalog")
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

    // Four items owned by ann read in two full pages of two, and the second
    // says no page follows. Catches a cursor issued whenever a page is
    // full, which hands the caller an empty last page.
    #[test]
    fn a_last_full_page_has_no_next() {
        let home = tempfile::tempdir().expect("temp dir");
        let store = stocked(home.path());
        assert_eq!(
            pages(&store, "by_owner", &[text("ann")], 2),
            [vec![1, 4], vec![5, 7]]
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

    // With the fixture analyzed, every statement `find` can run, on each
    // declared index, with each length of equality prefix, from the start
    // and after a cursor, plans as exactly one search of that index that
    // uses the whole equality prefix and, after a cursor, that seek's one
    // range. The expected plans are written out by hand. Catches a scan, a
    // temporary sort, a prefix the index does not use, and a continuation
    // that filters rows instead of seeking past them.
    #[test]
    fn every_find_statement_is_one_search_of_its_index() {
        let home = tempfile::tempdir().expect("temp dir");
        let store = stocked(home.path());
        store
            .write(|tx| tx.execute_batch("ANALYZE").map_err(sql))
            .expect("analyze");
        let table = store.views().table("item").expect("item");
        let search = |index: &str, terms: &str| {
            format!(
                "SEARCH v_item_3 USING INDEX v_item_3__{index} (project_id=? AND generation=?{terms})"
            )
        };
        let cases: [(&str, usize, bool, &[&str]); 10] = [
            ("by_state_rank", 0, false, &[""]),
            (
                "by_state_rank",
                0,
                true,
                &[
                    " AND i_state=? AND i_rank=? AND k_id>?",
                    " AND i_state=? AND i_rank<?",
                    " AND i_state>?",
                ],
            ),
            ("by_state_rank", 1, false, &[" AND i_state=?"]),
            (
                "by_state_rank",
                1,
                true,
                &[
                    " AND i_state=? AND i_rank=? AND k_id>?",
                    " AND i_state=? AND i_rank<?",
                ],
            ),
            ("by_state_rank", 2, false, &[" AND i_state=? AND i_rank=?"]),
            (
                "by_state_rank",
                2,
                true,
                &[" AND i_state=? AND i_rank=? AND k_id>?"],
            ),
            ("by_owner", 0, false, &[""]),
            (
                "by_owner",
                0,
                true,
                &[" AND i_owner=? AND k_id>?", " AND i_owner>?"],
            ),
            ("by_owner", 1, false, &[" AND i_owner=?"]),
            ("by_owner", 1, true, &[" AND i_owner=? AND k_id>?"]),
        ];
        for (name, fixed, after, expected) in cases {
            let index = table.index(name).expect("index");
            let position = some_position(table, index, fixed);
            let seeks = table.seeks(index, fixed, after.then_some(position.as_slice()));
            let plans: Vec<Vec<String>> = seeks
                .iter()
                .map(|seek| {
                    let params = seek.params(&project(), 0, &some_equals(index, fixed), 4);
                    store
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
                        .expect("plan")
                })
                .collect();
            let expected: Vec<Vec<String>> = expected
                .iter()
                .map(|terms| vec![search(name, terms)])
                .collect();
            assert_eq!(plans, expected, "{name}, {fixed} fixed, after: {after}");
        }
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
        store
            .write(|tx| {
                tx.execute(
                    "INSERT INTO project (project_id, name, created_at) VALUES ('p2', 'two', ?1)",
                    [AT],
                )
                .map_err(sql)?;
                Ok(())
            })
            .expect("project");
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
                ProjectId("p2".into()),
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

    // A cursor issued while generation 0 was live is refused once the
    // project reads generation 1. Catches a cursor that carries only a
    // position, which would page the new generation from the old one's
    // place.
    #[test]
    fn a_cursor_from_another_generation_is_refused() {
        let home = tempfile::tempdir().expect("temp dir");
        let store = stocked(home.path());
        let cursor = store
            .find(&project(), "item", &query("by_state_rank", &[], 2, None))
            .expect("find")
            .next
            .expect("a next page");
        flip_to_generation_1(&store);
        assert_eq!(
            store.find(
                &project(),
                "item",
                &query("by_state_rank", &[], 2, Some(cursor))
            ),
            Err(StoreError::Refused(Refusal::InvalidCursor))
        );
    }

    // A second put at the same key replaces the first: `get` returns the
    // second body and sequence, and the view holds one row. Catches a put
    // that inserts beside the old row or keeps the old body.
    #[test]
    fn a_second_put_replaces_the_first() {
        let home = tempfile::tempdir().expect("temp dir");
        let store = open(home.path());
        put(&store, 1, "open", 5, "ann", 11);
        put(&store, 1, "done", 8, "bob", 20);
        let got = store
            .get(&project(), "item", &key(1))
            .expect("get")
            .expect("present");
        // Compared parsed: the text's key order depends on serde_json's
        // features, which the workspace build unifies.
        let second = r#"{"state": "done", "id": 1, "owner": "bob", "rank": 8}"#
            .parse()
            .expect("json");
        assert_eq!((got.produced_seq, got.body), (20, second));
        let rows: i64 = raw(home.path())
            .query_row("SELECT count(*) FROM v_item_3", [], |row| row.get(0))
            .expect("count");
        assert_eq!(rows, 1);
    }

    // `get` returns the sequence of the event that put the document and
    // the view's projector version, 3 in the fixture spec. Catches
    // provenance not stored, or not read back.
    #[test]
    fn get_returns_the_documents_provenance() {
        let home = tempfile::tempdir().expect("temp dir");
        let store = open(home.path());
        put(&store, 9, "open", 1, "ann", 42);
        let got = store
            .get(&project(), "item", &key(9))
            .expect("get")
            .expect("present");
        assert_eq!((got.produced_seq, got.projector_version), (42, 3));
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
        flip_to_generation_1(&store);
        put(&store, 2, "open", 5, "ann", 12);
        assert_eq!(store.get(&project(), "item", &key(1)), Ok(None));
        assert!(matches!(
            store.get(&project(), "item", &key(2)),
            Ok(Some(_))
        ));
        let generations: Vec<(i64, i64)> = store
            .read(|conn| {
                let mut statement =
                    conn.prepare("SELECT k_id, generation FROM v_item_3 ORDER BY k_id")?;
                statement
                    .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?
                    .collect()
            })
            .expect("rows");
        assert_eq!(generations, [(1, 0), (2, 1)]);
    }

    // Writes into generation 1 while generation 0 is live, a changed item
    // and a new one, leave the live reads as they were. Catches a write
    // that ignores the generation it is given and lands in the live one.
    #[test]
    fn a_write_to_another_generation_leaves_live_reads_unchanged() {
        let home = tempfile::tempdir().expect("temp dir");
        let store = open(home.path());
        put(&store, 1, "open", 5, "ann", 11);
        store
            .write(|tx| {
                let table = store.views().table("item")?;
                write_change(tx, table, &project(), 1, &item(1, "done", 8, "bob"), 20)?;
                write_change(tx, table, &project(), 1, &item(2, "open", 5, "ann"), 21)
            })
            .expect("write generation 1");
        let live = store
            .get(&project(), "item", &key(1))
            .expect("get")
            .expect("present");
        let first = r#"{"state": "open", "id": 1, "owner": "ann", "rank": 5}"#
            .parse()
            .expect("json");
        assert_eq!((live.produced_seq, live.body), (11, first));
        assert_eq!(store.get(&project(), "item", &key(2)), Ok(None));
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

    // A body whose id differs from its key, lacks the id, or holds it as
    // text is refused and writes nothing. Catches a key column that
    // disagrees with the document it indexes.
    #[test]
    fn a_document_whose_key_fields_differ_from_its_key_is_refused() {
        let home = tempfile::tempdir().expect("temp dir");
        let store = open(home.path());
        for body in [
            r#"{"id": 2, "state": "open", "rank": 5, "owner": "ann"}"#,
            r#"{"state": "open", "rank": 5, "owner": "ann"}"#,
            r#"{"id": "1", "state": "open", "rank": 5, "owner": "ann"}"#,
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
        assert_eq!(store.get(&project(), "item", &key(2)), Ok(None));
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

    // Changing any one declared part of the spec changes its catalog text.
    // Catches a rendering that leaves out a field, so a spec changed there
    // without a new version would pass the catalog check.
    #[test]
    fn the_catalog_text_changes_with_every_part_of_the_spec() {
        let base = render(&item_view());
        let mut variants: Vec<(&str, ViewSpec)> = Vec::new();
        let mut changed = |what, change: fn(&mut ViewSpec)| {
            let mut spec = item_view();
            change(&mut spec);
            variants.push((what, spec));
        };
        changed("view name", |spec| spec.name = "thing".into());
        changed("version", |spec| spec.version = 4);
        changed("key name", |spec| spec.key[0].name = "number".into());
        changed("key kind", |spec| spec.key[0].kind = FieldKind::Text);
        changed("key added", |spec| {
            spec.key.push(FieldSpec {
                name: "part".into(),
                kind: FieldKind::Integer,
            })
        });
        changed("index name", |spec| spec.indexes[1].name = "by_who".into());
        changed("index removed", |spec| {
            spec.indexes.pop();
        });
        changed("index field name", |spec| {
            spec.indexes[1].fields[0].name = "who".into()
        });
        changed("index field kind", |spec| {
            spec.indexes[0].fields[1].kind = FieldKind::Text
        });
        changed("index field order", |spec| {
            spec.indexes[0].fields[1].order = Order::Ascending
        });
        changed("index field added", |spec| {
            spec.indexes[1]
                .fields
                .push(index_field("rank", FieldKind::Integer, Order::Ascending))
        });
        changed("page bound", |spec| spec.page_bound = 4);
        for (what, spec) in variants {
            assert_ne!(render(&spec), base, "{what}");
        }
    }

    // Reopening with the same version but another page bound is refused,
    // and the catalog keeps the one version it had. Catches a changed spec
    // read through the table the old spec made.
    #[test]
    fn a_changed_spec_under_the_same_version_is_refused() {
        let home = tempfile::tempdir().expect("temp dir");
        drop(open_with(home.path(), vec![item_view()]).expect("open"));
        let changed = ViewSpec {
            page_bound: 4,
            ..item_view()
        };
        assert!(matches!(
            open_with(home.path(), vec![changed]),
            Err(StoreError::Refused(Refusal::MalformedKey { view, .. })) if view == "item"
        ));
        assert_eq!(catalog(&raw(home.path())), [("item".into(), 3)]);
    }

    // A new version of a view gets its own table and indexes beside the
    // old version's, and its own catalog row. Catches a new version that
    // reuses the table the old spec made.
    #[test]
    fn a_new_version_gets_its_own_table() {
        let home = tempfile::tempdir().expect("temp dir");
        drop(open_with(home.path(), vec![item_view()]).expect("open"));
        let next = ViewSpec {
            version: 4,
            ..item_view()
        };
        drop(open_with(home.path(), vec![next]).expect("open version 4"));
        let conn = raw(home.path());
        assert_eq!(
            view_names(&conn),
            [
                "v_item_3",
                "v_item_3__by_owner",
                "v_item_3__by_state_rank",
                "v_item_4",
                "v_item_4__by_owner",
                "v_item_4__by_state_rank",
            ]
        );
        assert_eq!(catalog(&conn), [("item".into(), 3), ("item".into(), 4)]);
    }

    // With every table, index and catalog row in place nothing is pending;
    // a dropped index or a missing catalog row is. Catches an open that
    // skips a missing index, and one that always takes the write path.
    #[test]
    fn only_a_missing_part_makes_a_view_pending() {
        let home = tempfile::tempdir().expect("temp dir");
        let store = open(home.path());
        let pending = || store.snapshot(|conn| store.views().pending(conn));
        assert_eq!(pending(), Ok(false));
        let conn = raw(home.path());
        conn.execute_batch("DROP INDEX v_item_3__by_owner")
            .expect("drop");
        assert_eq!(pending(), Ok(true));
        drop(open_with(home.path(), vec![item_view()]).expect("reopen"));
        assert_eq!(pending(), Ok(false));
        conn.execute("DELETE FROM view_catalog", [])
            .expect("delete");
        assert_eq!(pending(), Ok(true));
    }

    // While another connection holds the database's write lock, a store
    // whose views are all in place still opens. Catches an open that
    // begins a write transaction on every call: it would wait out the busy
    // timeout and fail with `Busy`.
    #[test]
    fn opening_with_views_in_place_takes_no_write_transaction() {
        let home = tempfile::tempdir().expect("temp dir");
        drop(open_with(home.path(), vec![item_view()]).expect("open"));
        let holder = raw(home.path());
        holder.execute_batch("BEGIN IMMEDIATE").expect("hold");
        let reopened = open_with(home.path(), vec![item_view()]);
        holder.execute_batch("ROLLBACK").expect("release");
        assert!(reopened.is_ok(), "{:?}", reopened.err());
    }

    // A store a newer binary stamped opens read-only and gets no view
    // tables and no catalog rows. Catches view creation that skips the
    // epoch rule.
    #[test]
    fn a_read_only_store_creates_no_view_table() {
        let home = tempfile::tempdir().expect("temp dir");
        drop(SqliteStore::open(home.path(), AT, Options::default()).expect("open"));
        let conn = raw(home.path());
        conn.execute("UPDATE schema_meta SET value = 2 WHERE key = 'epoch'", [])
            .expect("stamp");
        drop(open_with(home.path(), vec![item_view()]).expect("open read-only"));
        assert!(view_names(&conn).is_empty());
        assert!(catalog(&conn).is_empty());
    }
}
