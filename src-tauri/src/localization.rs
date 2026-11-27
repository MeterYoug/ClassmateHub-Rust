use serde_json::Value;
use std::sync::OnceLock;

const MESSAGES: &str = include_str!("../../src/locales/messages.json");

pub fn dialog_text(locale: Option<&str>, source: &'static str) -> &'static str {
    let Some(locale @ ("en" | "zh-Hant" | "ja" | "ko")) = locale else {
        return source;
    };
    static CATALOGS: OnceLock<Option<Value>> = OnceLock::new();
    CATALOGS
        .get_or_init(|| serde_json::from_str(MESSAGES).ok())
        .as_ref()
        .and_then(|catalogs| catalogs.get(locale)?.get(source)?.as_str())
        .unwrap_or(source)
}

#[cfg(test)]
mod tests {
    use super::dialog_text;

    #[test]
    fn desktop_dialogs_use_the_requested_locale_and_fall_back_to_simplified_chinese() {
        let source = "导出班伴备份";
        for (locale, expected) in [
            ("en", "Export ClassmateHub backup"),
            ("zh-Hant", "匯出班伴備份"),
            ("ja", "ClassmateHub のバックアップを出力"),
            ("ko", "ClassmateHub 백업 내보내기"),
        ] {
            assert_eq!(dialog_text(Some(locale), source), expected);
        }
        assert_eq!(dialog_text(None, source), source);
        assert_eq!(dialog_text(Some("es"), source), source);
    }

    #[test]
    fn workbook_save_dialog_text_is_available_in_all_four_additional_locales() {
        for source in [
            "另存为成绩表",
            "另存为成绩导入模板",
            "另存为基础资料",
            "考试成绩.xlsx",
            "成绩导入模板.xlsx",
            "{0}资料.xlsx",
        ] {
            assert_eq!(dialog_text(None, source), source);
            for locale in ["en", "zh-Hant", "ja", "ko"] {
                assert_ne!(dialog_text(Some(locale), source), source, "{locale}: {source}");
            }
        }
    }
}
