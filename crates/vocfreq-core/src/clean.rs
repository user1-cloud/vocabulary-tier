//! 文本清洗：剥离 HTML 标签与实体、还原被双重转义的空白、规范化空白。
//!
//! 论坛语料的 `回复` 字段是 HTML 片段，实测含 `<P>`、`<BR>`、`<img>`、
//! `<div class='posttime'>`，以及被双重转义的 `\\r\\n`（JSON 解码后是
//! 反斜杠 + r 两个字面字符）。不清洗的话标签名和属性会被当成词统计。

/// 是否块级标签（遇到时插入分隔符，避免相邻块的内容被粘连成一个词）。
fn is_block_tag(name: &str) -> bool {
    matches!(
        name,
        "br" | "p"
            | "div"
            | "li"
            | "ul"
            | "ol"
            | "tr"
            | "td"
            | "th"
            | "table"
            | "h1"
            | "h2"
            | "h3"
            | "h4"
            | "h5"
            | "h6"
            | "blockquote"
            | "pre"
            | "hr"
            | "section"
            | "article"
            | "header"
            | "footer"
            | "nav"
            | "dl"
            | "dt"
            | "dd"
    )
}

/// 解码 HTML 实体，返回 (字符, 消耗字节数)。不是实体则返回 None。
fn decode_entity(s: &str) -> Option<(char, usize)> {
    let b = s.as_bytes();
    if b.len() < 3 || b[0] != b'&' {
        return None;
    }
    // 命名实体
    const NAMED: [(&str, char); 8] = [
        ("&amp;", '&'),
        ("&lt;", '<'),
        ("&gt;", '>'),
        ("&quot;", '"'),
        ("&apos;", '\''),
        ("&nbsp;", ' '),
        ("&#39;", '\''),
        ("&ensp;", ' '),
    ];
    for (pat, ch) in NAMED {
        if s.len() >= pat.len() && s.as_bytes()[..pat.len()].eq_ignore_ascii_case(pat.as_bytes()) {
            return Some((ch, pat.len()));
        }
    }
    // 数字实体 &#123; / &#x1F600;
    if b.len() > 3 && b[1] == b'#' {
        let hex = b[2] == b'x' || b[2] == b'X';
        let start = if hex { 3 } else { 2 };
        let mut end = start;
        while end < b.len() && b[end] != b';' && end - start < 10 {
            end += 1;
        }
        if end < b.len() && b[end] == b';' {
            let digits = &s[start..end];
            if let Ok(code) = u32::from_str_radix(digits, if hex { 16 } else { 10 }) {
                if let Some(c) = char::from_u32(code) {
                    return Some((c, end + 1));
                }
            }
        }
    }
    None
}

/// 向 `out` 追加一个分隔空格（避免重复和行首空格）。
#[inline]
fn push_sep(out: &mut String) {
    if !out.is_empty() && !out.ends_with(' ') {
        out.push(' ');
    }
}

/// 字节级 ASCII 大小写不敏感的前缀判断（不涉及字符边界，绝不会 panic）。
#[inline]
fn starts_with_ignore_ascii_case(hay: &[u8], needle: &[u8]) -> bool {
    hay.len() >= needle.len() && hay[..needle.len()].eq_ignore_ascii_case(needle)
}

/// 字节级 ASCII 大小写不敏感的子串查找。只在遇到 `<script` / `<style` 时调用，
/// 频率极低，因此朴素实现足够。
fn find_ignore_ascii_case(hay: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || hay.len() < needle.len() {
        return None;
    }
    (0..=hay.len() - needle.len()).find(|&i| hay[i..i + needle.len()].eq_ignore_ascii_case(needle))
}

/// 清洗 `src` 并追加到 `out`。
///
/// * `strip_html` —— 是否剥离标签（论坛为 true，其余为 false）
///
/// 无论哪种模式都会：
/// * 还原形如 `\r` `\n` `\t` 的**字面**两字符序列（论坛里双重转义的残留）；
/// * 把 CR/LF/Tab 与控制字符统一成单个空格；
/// * 折叠连续空白。
pub fn clean_into(src: &str, strip_html: bool, out: &mut String) {
    let b = src.as_bytes();
    let mut i = 0usize;

    while i < b.len() {
        // 字面转义序列 \r \n \t \\（论坛双重转义的残留）
        if b[i] == b'\\' && i + 1 < b.len() {
            let n = b[i + 1];
            if matches!(n, b'r' | b'n' | b't' | b'\\') {
                push_sep(out);
                i += 2;
                continue;
            }
        }

        if strip_html && b[i] == b'<' {
            // 注释
            if src[i..].starts_with("<!--") {
                match src[i + 4..].find("-->") {
                    Some(p) => {
                        i = i + 4 + p + 3;
                        push_sep(out);
                    }
                    None => i = b.len(),
                }
                continue;
            }
            // script / style 的内容一并丢弃。
            // 注意这里必须走**字节**比较：按字符切片取前 8 字节会踩到多字节字符中间
            // 而 panic（例如 `<P>灿烂…` 的第 8 字节落在「烂」里面）。
            let rest = &b[i..];
            let skip_until: Option<&[u8]> = if starts_with_ignore_ascii_case(rest, b"<script") {
                Some(b"</script")
            } else if starts_with_ignore_ascii_case(rest, b"<style") {
                Some(b"</style")
            } else {
                None
            };
            if let Some(close) = skip_until {
                match find_ignore_ascii_case(rest, close) {
                    Some(p) => {
                        i += p + close.len();
                        while i < b.len() && b[i] != b'>' {
                            i += 1;
                        }
                        if i < b.len() {
                            i += 1;
                        }
                    }
                    None => i = b.len(),
                }
                push_sep(out);
                continue;
            }
            // 普通标签：找到 '>'，跳过引号内的内容
            let mut j = i + 1;
            let mut quote = 0u8;
            while j < b.len() {
                let c = b[j];
                if quote != 0 {
                    if c == quote {
                        quote = 0;
                    }
                } else if c == b'"' || c == b'\'' {
                    quote = c;
                } else if c == b'>' {
                    break;
                }
                j += 1;
            }
            let inner = &src[i + 1..j.min(b.len())];
            let name: String = inner
                .trim_start_matches('/')
                .trim_start()
                .chars()
                .take_while(|c| c.is_ascii_alphanumeric())
                .collect::<String>()
                .to_ascii_lowercase();
            if is_block_tag(&name) {
                push_sep(out);
            }
            i = if j < b.len() { j + 1 } else { b.len() };
            continue;
        }

        if b[i] == b'&' {
            if let Some((ch, len)) = decode_entity(&src[i..]) {
                if ch == ' ' {
                    push_sep(out);
                } else {
                    out.push(ch);
                }
                i += len;
                continue;
            }
        }

        // 普通字符
        let ch = src[i..].chars().next().unwrap();
        if ch.is_whitespace() || ch.is_control() {
            push_sep(out);
        } else {
            out.push(ch);
        }
        i += ch.len_utf8();
    }

    // 收尾去掉尾部空白：分隔符是按需插入的，末尾可能残留一个空格
    while out.ends_with(' ') {
        out.pop();
    }
}

/// 清洗成新字符串。
pub fn clean(src: &str, strip_html: bool) -> String {
    let mut out = String::with_capacity(src.len());
    clean_into(src, strip_html, &mut out);
    out
}

/// 该字符是否汉字（基本区 + 扩展 A/B~F + 兼容表意文字）。
#[inline]
pub fn is_cjk_ideograph(c: char) -> bool {
    matches!(c as u32,
        0x3400..=0x4DBF        // 扩展 A
        | 0x4E00..=0x9FFF      // 基本区
        | 0xF900..=0xFAFF      // 兼容表意文字
        | 0x20000..=0x2FA1F    // 扩展 B ~ F
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_tags_and_entities() {
        let src = r#"<P>灿烂的星空 <BR><BR>有执着</P><div class='posttime'>====该贴于[2008-05-10]被编辑====</div>"#;
        let got = clean(src, true);
        assert!(!got.contains('<'), "标签应被剥掉: {got}");
        assert!(!got.contains("div"));
        assert!(got.contains("灿烂的星空"));
        assert!(got.contains("有执着"));
        assert!(got.contains("该贴于"));
    }

    #[test]
    fn drops_script_and_style_contents() {
        let src =
            "<p>前</p><script>var x = '<b>坏</b>';</script><style>.a{color:red}</style><p>后</p>";
        let got = clean(src, true);
        assert!(got.contains('前') && got.contains('后'));
        assert!(!got.contains("var x"), "script 内容应被丢弃: {got}");
        assert!(!got.contains("color"), "style 内容应被丢弃: {got}");
    }

    #[test]
    fn decodes_entities() {
        let got = clean("A&amp;B&nbsp;C&lt;D&gt;&#39;E&#x4E2D;", false);
        assert_eq!(got, "A&B C<D>'E中");
    }

    #[test]
    fn handles_literal_backslash_escapes() {
        let got = clean(r"你好\r\n世界\t再次", false);
        assert_eq!(got, "你好 世界 再次");
    }

    #[test]
    fn collapses_whitespace_and_never_starts_with_space() {
        let got = clean("  a \n\n b \t c  ", false);
        assert_eq!(got, "a b c");
    }

    #[test]
    fn quote_inside_attribute_does_not_end_tag_early() {
        let src = r#"<a href="http://x/?a=1>b=2" target=_blank>链接</a>"#;
        let got = clean(src, true);
        assert!(got.contains("链接"));
        assert!(!got.contains("target"));
    }

    #[test]
    fn cjk_classification() {
        assert!(is_cjk_ideograph('中'));
        assert!(is_cjk_ideograph('龘'));
        assert!(!is_cjk_ideograph('a'));
        assert!(!is_cjk_ideograph('，'));
        assert!(!is_cjk_ideograph('1'));
    }
}
