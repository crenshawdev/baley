//! The shell command that fetches one provider's model list for
//! `baley models import` (design 0003 section 5). Baley prints it and never
//! runs or expands it, so the key stays in the owner's environment.

use baley_core::catalog::Provider;

use super::trigger::{key_name, list_address};
use super::{DEADLINE_SECONDS, MAX_BODY_BYTES};

/// The POSIX `sh` pipeline that fetches `provider`'s model list and pipes
/// it, with its `baley-fetch` trailer, into `baley models import`.
///
/// - `--disable` must be curl's first argument: it stops curl reading the
///   owner's `.curlrc`, where `location` or `fail` could follow a redirect
///   or turn an HTTP status into a transport failure.
/// - The header line reaches curl on stdin through the `printf` builtin and
///   `--header @-`, so the key is never in curl's arguments or in `ps`. The
///   variable is named, never expanded by Baley.
/// - `--proto =https` allows https only, and with no `--location` no
///   redirect is followed.
/// - `--max-time` bounds the whole fetch, body included, and
///   `--max-filesize` is the body bound `import` also checks.
/// - The body goes to fd 3, the pipe into `import`, and `--write-out` hands
///   the status to the shell. After curl exits the shell writes the trailer
///   on a line of its own: exit 0 is `complete=true`, exit 63 (curl's size
///   limit) is `complete=false`, and any other exit is `transport-failed`.
pub fn fetch_command(provider: Provider) -> String {
    let variable = key_name(provider);
    let address = list_address(provider);
    let name = provider.name();
    format!(
        "{{ s=$(printf 'Authorization: Bearer %s\\n' \"${variable}\" \
         | curl --disable --silent --show-error --proto =https \
         --max-time {DEADLINE_SECONDS} --max-filesize {MAX_BODY_BYTES} --header @- \
         --output /dev/fd/3 --write-out '%{{http_code}}' {address}); e=$?; \
         if [ \"$e\" -eq 0 ]; then printf '\\nbaley-fetch status=%s complete=true\\n' \"$s\"; \
         elif [ \"$e\" -eq 63 ]; then printf '\\nbaley-fetch status=%s complete=false\\n' \"$s\"; \
         else printf '\\nbaley-fetch transport-failed\\n'; fi; }} 3>&1 \
         | baley models import {name}"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_openai_fetch_command_keeps_the_key_off_curls_arguments_and_bounds_the_fetch() {
        assert_eq!(
            fetch_command(Provider::OpenAi),
            r#"{ s=$(printf 'Authorization: Bearer %s\n' "$OPENAI_API_KEY" | curl --disable --silent --show-error --proto =https --max-time 20 --max-filesize 4194304 --header @- --output /dev/fd/3 --write-out '%{http_code}' https://api.openai.com/v1/models); e=$?; if [ "$e" -eq 0 ]; then printf '\nbaley-fetch status=%s complete=true\n' "$s"; elif [ "$e" -eq 63 ]; then printf '\nbaley-fetch status=%s complete=false\n' "$s"; else printf '\nbaley-fetch transport-failed\n'; fi; } 3>&1 | baley models import openai"#
        );
    }

    #[test]
    fn the_deepseek_fetch_command_names_deepseeks_address_variable_and_import() {
        assert_eq!(
            fetch_command(Provider::DeepSeek),
            r#"{ s=$(printf 'Authorization: Bearer %s\n' "$DEEPSEEK_API_KEY" | curl --disable --silent --show-error --proto =https --max-time 20 --max-filesize 4194304 --header @- --output /dev/fd/3 --write-out '%{http_code}' https://api.deepseek.com/models); e=$?; if [ "$e" -eq 0 ]; then printf '\nbaley-fetch status=%s complete=true\n' "$s"; elif [ "$e" -eq 63 ]; then printf '\nbaley-fetch status=%s complete=false\n' "$s"; else printf '\nbaley-fetch transport-failed\n'; fi; } 3>&1 | baley models import deepseek"#
        );
    }
}
