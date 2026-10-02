//! 语料库 JSON 规则：内置规则、自动探测、通用抽取。
//!
//! 实测你的语料库只有 4 条规则（`docs/DESIGN.md` §2）：
//! `段落[].内容` / `回复[].回复`（含 HTML）/ `text` / `zh_text`。
//!
//! 抽取走 `DeserializeSeed`，只在命中目标字段时收字符串，其余 `IgnoredAny`；
//! 无转义时借用原缓冲区，零拷贝。

use std::borrow::Cow;

use serde::de::{DeserializeSeed, Deserializer, IgnoredAny, MapAccess, SeqAccess, Visitor};
use serde::{Deserialize, Serialize};

use crate::{Error, Result};

/// 一条语料库解析规则。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SourceRule {
    pub name: String,
    /// 顶层数组字段，例如 `段落`
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub array_key: Option<String>,
    /// 数组元素内取文本的字段，例如 `内容`
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text_key: Option<String>,
    /// 顶层直接取文本的字段，例如 `text` / `zh_text`
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plain_key: Option<String>,
    /// `plain_key` 为空字符串时依次回退的字段，例如 `cht_text`
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub alt_text_keys: Vec<String>,
    /// 除数组元素外，额外计入的顶层文本字段，例如论坛的 `主题`
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub top_text_keys: Vec<String>,
    /// 是否先剥离 HTML 标签与实体
    #[serde(default)]
    pub strip_html: bool,
    /// 命中该 glob 的文件强制使用本规则（可选）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub glob: Option<String>,
}

impl SourceRule {
    fn new(name: &str) -> Self {
        SourceRule {
            name: name.to_string(),
            array_key: None,
            text_key: None,
            plain_key: None,
            alt_text_keys: Vec::new(),
            top_text_keys: Vec::new(),
            strip_html: false,
            glob: None,
        }
    }

    /// 该规则可能由哪些顶层 key 命中（用于探测与回退匹配）。
    pub fn anchors(&self) -> Vec<&str> {
        let mut v = Vec::new();
        if let Some(a) = &self.array_key {
            v.push(a.as_str());
        }
        if let Some(p) = &self.plain_key {
            v.push(p.as_str());
        }
        v
    }
}

/// 内置规则，**顺序即探测优先级**。
///
/// `段落` 必须排在 `text` 之前：MNBVC 的段落型文件里也可能出现别的字段。
pub fn builtin_rules() -> Vec<SourceRule> {
    let mut paragraph = SourceRule::new("mnbvc_paragraph");
    paragraph.array_key = Some("段落".into());
    paragraph.text_key = Some("内容".into());

    let mut forum = SourceRule::new("mnbvc_forum");
    forum.array_key = Some("回复".into());
    forum.text_key = Some("回复".into());
    forum.top_text_keys = vec!["主题".into()];
    forum.strip_html = true;

    let mut subtitle = SourceRule::new("parallel_subtitle");
    subtitle.plain_key = Some("zh_text".into());
    subtitle.alt_text_keys = vec!["cht_text".into()];

    let mut plain = SourceRule::new("plain_text");
    plain.plain_key = Some("text".into());

    vec![paragraph, forum, subtitle, plain]
}

/// 收集顶层 key，用于探测。只需每文件跑一次，因此用简单的收集器即可。
struct KeySet(Vec<String>);

impl<'de> Deserialize<'de> for KeySet {
    fn deserialize<D: Deserializer<'de>>(d: D) -> std::result::Result<Self, D::Error> {
        struct V;
        impl<'de> Visitor<'de> for V {
            type Value = KeySet;
            fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str("一个 JSON 对象")
            }
            fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> std::result::Result<KeySet, A::Error> {
                let mut keys = Vec::new();
                while let Some(k) = map.next_key::<KeyStr<'de>>()? {
                    keys.push(k.0.into_owned());
                    map.next_value::<IgnoredAny>()?;
                }
                Ok(KeySet(keys))
            }
        }
        d.deserialize_map(V)
    }
}

/// 由一行样本自动选择规则。
///
/// 先匹配全部数组型规则，再匹配文本型规则，保证优先级与 `builtin_rules` 顺序一致。
pub fn detect<'r>(rules: &'r [SourceRule], sample: &[u8]) -> Result<&'r SourceRule> {
    let keys = match serde_json::from_slice::<KeySet>(sample) {
        Ok(KeySet(k)) => k,
        Err(e) => {
            return Err(Error::Schema(format!(
                "无法解析样本行以探测格式（前 120 字节: {:?}）: {e}",
                String::from_utf8_lossy(&sample[..sample.len().min(120)])
            )));
        }
    };
    let hit = |r: &SourceRule| r.anchors().iter().any(|a| keys.iter().any(|k| k == a));
    if let Some(r) = rules.iter().find(|r| r.array_key.is_some() && hit(r)) {
        return Ok(r);
    }
    if let Some(r) = rules.iter().find(|r| r.plain_key.is_some() && hit(r)) {
        return Ok(r);
    }
    Err(Error::Schema(format!(
        "无法识别的顶层字段: {keys:?}；可用 --rules 指定自定义规则"
    )))
}

/// JSON 对象键。
///
/// ⚠️ **绝不要用 `Cow<'de, str>` 做键**。serde 对 `Cow` 只有一条通用实现：
///
/// ```ignore
/// impl<'de, 'a, T> Deserialize<'de> for Cow<'a, T> {
///     fn deserialize<D>(d: D) -> Result<Self, D::Error> {
///         T::Owned::deserialize(d).map(Cow::Owned)   // 永远分配，从不借用
///     }
/// }
/// ```
///
/// 也就是说 `Cow<'de, str>` 并不是零拷贝，它每次都先造一个 `String`。而 MNBVC 的
/// 段落数组是 `[{行号, 是否重复, …, 内容}, …]`，一个 1GB 文件里有上百万个元素、
/// 每个元素 5 个键 —— 用 `Cow` 当键就等于上千万次堆分配。实测这是扫描阶段最大的
/// 单项开销（gov 域因此慢了 5.7 倍）。
///
/// 这里改用 `deserialize_str` + `visit_borrowed_str`：无转义时零拷贝，
/// 有转义时才退化成 Owned。
pub struct KeyStr<'de>(pub Cow<'de, str>);

impl<'de> Deserialize<'de> for KeyStr<'de> {
    fn deserialize<D: Deserializer<'de>>(d: D) -> std::result::Result<Self, D::Error> {
        struct V;
        impl<'de> Visitor<'de> for V {
            type Value = KeyStr<'de>;
            fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str("一个字符串键")
            }
            fn visit_borrowed_str<E>(self, v: &'de str) -> std::result::Result<Self::Value, E> {
                Ok(KeyStr(Cow::Borrowed(v)))
            }
            fn visit_str<E>(self, v: &str) -> std::result::Result<Self::Value, E> {
                Ok(KeyStr(Cow::Owned(v.to_string())))
            }
            fn visit_string<E>(self, v: String) -> std::result::Result<Self::Value, E> {
                Ok(KeyStr(Cow::Owned(v)))
            }
            /// serde_json 的键走的是字符串分支，这两个只是兜底，因此用 lossy 转换，
            /// 免去为了构造成 Error 而引入 `E: serde::de::Error` 约束。
            fn visit_borrowed_bytes<E>(self, v: &'de [u8]) -> std::result::Result<Self::Value, E> {
                Ok(KeyStr(Cow::Owned(String::from_utf8_lossy(v).into_owned())))
            }
            fn visit_bytes<E>(self, v: &[u8]) -> std::result::Result<Self::Value, E> {
                Ok(KeyStr(Cow::Owned(String::from_utf8_lossy(v).into_owned())))
            }
        }
        d.deserialize_str(V)
    }
}

/// 宽松地取一个可选字符串：字符串取之，null/数字/布尔/容器一律视为 None。
///
/// 用 `deserialize_any` 是因为这里要容忍各种类型；serde_json 对无转义的字符串会走
/// `visit_borrowed_str`，所以正常语料下是零拷贝。
struct OptStr<'de>(Option<Cow<'de, str>>);

impl<'de> Deserialize<'de> for OptStr<'de> {
    fn deserialize<D: Deserializer<'de>>(d: D) -> std::result::Result<Self, D::Error> {
        struct V;
        impl<'de> Visitor<'de> for V {
            type Value = OptStr<'de>;
            fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str("字符串或 null")
            }
            fn visit_borrowed_str<E>(self, v: &'de str) -> std::result::Result<Self::Value, E> {
                Ok(OptStr(Some(Cow::Borrowed(v))))
            }
            fn visit_str<E>(self, v: &str) -> std::result::Result<Self::Value, E> {
                Ok(OptStr(Some(Cow::Owned(v.to_string()))))
            }
            fn visit_string<E>(self, v: String) -> std::result::Result<Self::Value, E> {
                Ok(OptStr(Some(Cow::Owned(v))))
            }
            fn visit_unit<E>(self) -> std::result::Result<Self::Value, E> {
                Ok(OptStr(None))
            }
            fn visit_none<E>(self) -> std::result::Result<Self::Value, E> {
                Ok(OptStr(None))
            }
            fn visit_u64<E>(self, _: u64) -> std::result::Result<Self::Value, E> {
                Ok(OptStr(None))
            }
            fn visit_i64<E>(self, _: i64) -> std::result::Result<Self::Value, E> {
                Ok(OptStr(None))
            }
            fn visit_f64<E>(self, _: f64) -> std::result::Result<Self::Value, E> {
                Ok(OptStr(None))
            }
            fn visit_bool<E>(self, _: bool) -> std::result::Result<Self::Value, E> {
                Ok(OptStr(None))
            }
            fn visit_seq<A: SeqAccess<'de>>(self, mut a: A) -> std::result::Result<Self::Value, A::Error> {
                while a.next_element::<IgnoredAny>()?.is_some() {}
                Ok(OptStr(None))
            }
            fn visit_map<A: MapAccess<'de>>(self, mut a: A) -> std::result::Result<Self::Value, A::Error> {
                while a.next_entry::<IgnoredAny, IgnoredAny>()?.is_some() {}
                Ok(OptStr(None))
            }
        }
        d.deserialize_any(V)
    }
}

/// 遍历数组，只收每个元素里 `text_key`（或回退键）的字符串。
struct TextArraySeed<'r> {
    text_key: Option<&'r str>,
    alt: &'r [String],
}

impl<'de, 'r> DeserializeSeed<'de> for TextArraySeed<'r> {
    type Value = Vec<Cow<'de, str>>;

    fn deserialize<D: Deserializer<'de>>(self, d: D) -> std::result::Result<Self::Value, D::Error> {
        struct V<'r> {
            text_key: Option<&'r str>,
            alt: &'r [String],
        }
        impl<'de, 'r> Visitor<'de> for V<'r> {
            type Value = Vec<Cow<'de, str>>;
            fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str("一个对象数组")
            }
            fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> std::result::Result<Self::Value, A::Error> {
                let mut out = Vec::new();
                while let Some(item) = seq.next_element_seed(TextArrayItemSeed {
                    text_key: self.text_key,
                    alt: self.alt,
                })? {
                    if let Some(t) = item {
                        if !t.trim().is_empty() {
                            out.push(t);
                        }
                    }
                }
                Ok(out)
            }
        }
        d.deserialize_seq(V { text_key: self.text_key, alt: self.alt })
    }
}

struct TextArrayItemSeed<'r> {
    text_key: Option<&'r str>,
    alt: &'r [String],
}

impl<'de, 'r> DeserializeSeed<'de> for TextArrayItemSeed<'r> {
    type Value = Option<Cow<'de, str>>;

    fn deserialize<D: Deserializer<'de>>(self, d: D) -> std::result::Result<Self::Value, D::Error> {
        struct V<'r> {
            text_key: Option<&'r str>,
            alt: &'r [String],
        }
        impl<'de, 'r> Visitor<'de> for V<'r> {
            type Value = Option<Cow<'de, str>>;
            fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str("一个含文本字段的对象")
            }
            fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> std::result::Result<Self::Value, A::Error> {
                let mut primary: Option<Cow<'de, str>> = None;
                let mut fallback: Option<Cow<'de, str>> = None;
                while let Some(k) = map.next_key::<KeyStr<'de>>()? {
                    let ks: &str = &k.0;
                    if self.text_key == Some(ks) {
                        primary = map.next_value::<OptStr<'de>>()?.0;
                    } else if self.alt.iter().any(|a| a == ks) {
                        let v = map.next_value::<OptStr<'de>>()?.0;
                        if fallback.is_none() {
                            fallback = v;
                        }
                    } else {
                        map.next_value::<IgnoredAny>()?;
                    }
                }
                Ok(match primary {
                    Some(p) if !p.trim().is_empty() => Some(p),
                    _ => fallback,
                })
            }
            /// 论坛数据里存在空对象 `{}`，`visit_unit` 也当作无文本。
            fn visit_unit<E>(self) -> std::result::Result<Self::Value, E> {
                Ok(None)
            }
            fn visit_none<E>(self) -> std::result::Result<Self::Value, E> {
                Ok(None)
            }
        }
        d.deserialize_any(V { text_key: self.text_key, alt: self.alt })
    }
}

/// 逐行抽取文本的 seed。
pub struct RowSeed<'r> {
    pub rule: &'r SourceRule,
}

impl<'de, 'r> DeserializeSeed<'de> for RowSeed<'r> {
    type Value = Vec<Cow<'de, str>>;

    fn deserialize<D: Deserializer<'de>>(self, d: D) -> std::result::Result<Self::Value, D::Error> {
        struct V<'r> {
            rule: &'r SourceRule,
        }
        impl<'de, 'r> Visitor<'de> for V<'r> {
            type Value = Vec<Cow<'de, str>>;
            fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str("一个对象")
            }
            fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> std::result::Result<Self::Value, A::Error> {
                let rule = self.rule;
                let mut out: Vec<Cow<'de, str>> = Vec::new();
                // plain 规则的候选，按优先级排序后取第一个非空
                let mut plain: Vec<(u8, Cow<'de, str>)> = Vec::new();

                while let Some(k) = map.next_key::<KeyStr<'de>>()? {
                    let ks: &str = &k.0;
                    if rule.array_key.as_deref() == Some(ks) {
                        let items = map.next_value_seed(TextArraySeed {
                            text_key: rule.text_key.as_deref(),
                            alt: &rule.alt_text_keys,
                        })?;
                        out.extend(items);
                    } else if rule.top_text_keys.iter().any(|t| t == ks) {
                        if let Some(v) = map.next_value::<OptStr<'de>>()?.0 {
                            out.push(v);
                        }
                    } else if rule.plain_key.as_deref() == Some(ks) {
                        if let Some(v) = map.next_value::<OptStr<'de>>()?.0 {
                            plain.push((0, v));
                        }
                    } else if let Some(pos) = rule.alt_text_keys.iter().position(|a| a == ks) {
                        if let Some(v) = map.next_value::<OptStr<'de>>()?.0 {
                            plain.push((1 + pos as u8, v));
                        }
                    } else {
                        map.next_value::<IgnoredAny>()?;
                    }
                }

                if !out.is_empty() {
                    return Ok(out);
                }
                plain.sort_by_key(|(p, _)| *p);
                for (_, v) in plain {
                    if !v.trim().is_empty() {
                        return Ok(vec![v]);
                    }
                }
                Ok(out)
            }
        }
        d.deserialize_map(V { rule: self.rule })
    }
}

/// 从一行的字节里按规则抽出全部文本。
///
/// 快路径用 `from_slice` 零拷贝；非法 UTF-8 时退到 `from_utf8_lossy` 抢救本行，
/// 失败则返回 `Ok(None)`（调用方应把它计为坏行并**继续**，绝不中断扫描）。
///
/// 注意 lossy 分支：`from_str` 借的是局部 `String`，生命周期比 `'de` 短，因此必须
/// 立刻把结果全部转成 `Cow::Owned` 才能返回。
pub fn extract_row<'de>(rule: &SourceRule, raw: &'de [u8]) -> Result<Option<Vec<Cow<'de, str>>>> {
    let mut de = serde_json::Deserializer::from_slice(raw);
    // 结构体字面量在这里必须加括号，否则 `RowSeed { rule }` 的 `{` 会被当成语句块
    if let Ok(v) = (RowSeed { rule }).deserialize(&mut de) {
        return Ok(Some(v));
    }

    let lossy = String::from_utf8_lossy(raw);
    let mut de = serde_json::Deserializer::from_str(&lossy);
    match (RowSeed { rule }).deserialize(&mut de) {
        Ok(v) => Ok(Some(
            v.into_iter().map(|c| Cow::Owned(c.into_owned())).collect::<Vec<_>>(),
        )),
        Err(_) => Ok(None),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detect_picks_paragraph_rule() {
        let rules = builtin_rules();
        let line = r#"{"文件名":"a","段落":[{"行号":1,"内容":"hi"}],"时间":"2023"}"#.as_bytes();
        assert_eq!(detect(&rules, line).unwrap().name, "mnbvc_paragraph");
    }

    #[test]
    fn detect_picks_forum_before_paragraph() {
        let rules = builtin_rules();
        let line = r#"{"ID":"1","主题":"t","回复":[{"楼ID":"1","回复":"<P>hi</P>"}]}"#.as_bytes();
        assert_eq!(detect(&rules, line).unwrap().name, "mnbvc_forum");
    }

    #[test]
    fn detect_picks_subtitle_and_plain() {
        let rules = builtin_rules();
        let sub = r#"{"en_text":"a","zh_text":"你好","cht_text":"妳好"}"#.as_bytes();
        assert_eq!(detect(&rules, sub).unwrap().name, "parallel_subtitle");
        let plain = r#"{"text":"正文","meta":{}}"#.as_bytes();
        assert_eq!(detect(&rules, plain).unwrap().name, "plain_text");
    }

    #[test]
    fn extract_paragraph_content() {
        let rules = builtin_rules();
        let r = &rules[0];
        let line = r#"{"段落":[{"内容":"ab"},{"内容":"cd"},{"扩展字段":""}]}"#.as_bytes();
        let got = extract_row(r, line).unwrap().unwrap();
        assert_eq!(got, vec!["ab", "cd"]);
    }

    #[test]
    fn extract_forum_includes_topic_and_skips_empty() {
        let rules = builtin_rules();
        let r = rules.iter().find(|r| r.name == "mnbvc_forum").unwrap();
        let line = r#"{"主题":"题目","回复":[{"回复":"<P>x</P>"},{}]}"#.as_bytes();
        let got = extract_row(r, line).unwrap().unwrap();
        assert_eq!(got, vec!["题目", "<P>x</P>"]);
    }

    #[test]
    fn extract_subtitle_falls_back_to_cht() {
        let rules = builtin_rules();
        let r = rules.iter().find(|r| r.name == "parallel_subtitle").unwrap();
        let line = r#"{"zh_text":"","cht_text":"繁體"}"#.as_bytes();
        let got = extract_row(r, line).unwrap().unwrap();
        assert_eq!(got, vec!["繁體"]);
    }

    #[test]
    fn extract_survives_invalid_utf8() {
        let rules = builtin_rules();
        let r = &rules[0];
        // "内容" 的值里塞入一个非法字节 0x8d
        let mut line = Vec::new();
        line.extend_from_slice(r#"{"段落":[{"内容":"a"#.as_bytes());
        line.push(0x8d);
        line.extend_from_slice(r#"b"}]}"#.as_bytes());
        let got = extract_row(r, &line).unwrap();
        assert!(got.is_some(), "非法 UTF-8 的坏行应被抢救而不是丢弃");
    }

    #[test]
    fn extract_ignores_non_string_values() {
        let rules = builtin_rules();
        let r = rules.iter().find(|r| r.name == "plain_text").unwrap();
        let line = br#"{"text":null,"meta":{"a":1}}"#;
        assert_eq!(extract_row(r, line).unwrap().unwrap().len(), 0);
    }
}
