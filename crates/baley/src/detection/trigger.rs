//! What a provider's model-list request is made of, and the command kind an
//! import records under (design 0003 section 6, CFG-R20). Baley names the
//! key's environment variable and never reads it.

use baley_core::catalog::Provider;

/// The command kind `baley models import` records its provider under.
pub const IMPORT_COMMAND: &str = "models.import";

/// The environment variable the fetch command reads `provider`'s key from.
pub fn key_name(provider: Provider) -> &'static str {
    match provider {
        Provider::OpenAi => "OPENAI_API_KEY",
        Provider::DeepSeek => "DEEPSEEK_API_KEY",
    }
}

/// The address of `provider`'s model list.
pub fn list_address(provider: Provider) -> &'static str {
    match provider {
        Provider::OpenAi => "https://api.openai.com/v1/models",
        Provider::DeepSeek => "https://api.deepseek.com/models",
    }
}
