//! 保存するファイル名。連番は「いま保存先にある一番大きい番号 + 1」。
//! 番号を別のところに覚えない (消したり、フォルダを移したりしても、ずれない)。

/// 名前の形。{n} は連番、{date} は 日付、{time} は 時刻。
/// 例: "shot_{date}_{n}" → "shot_20261008_003"
pub fn render(template: &str, n: u32, digits: usize, date: &str, time: &str) -> String {
    template
        .replace("{n}", &format!("{:0width$}", n, width = digits))
        .replace("{date}", date)
        .replace("{time}", time)
}

/// ファイル名の一覧から、template に合うものの一番大きい連番を探して、次の番号を返す。
/// 合うものが無ければ 1。拡張子は見ない (png でも jpg でも同じ番号列)。
pub fn next_number(template: &str, existing: &[String], date: &str, time: &str) -> u32 {
    // {n} の前後を取り出す。{n} が無い形なら、連番は使わない。
    let probe = render(template, 0, 1, date, time);
    let key = "{n}";
    let Some(pos) = template.find(key) else { return 1 };
    let prefix = render(&template[..pos], 0, 1, date, time);
    let suffix = render(&template[pos + key.len()..], 0, 1, date, time);
    let _ = probe;
    let mut max = 0u32;
    for name in existing {
        let stem = match name.rfind('.') { Some(i) => &name[..i], None => name.as_str() };
        if let Some(rest) = stem.strip_prefix(prefix.as_str()) {
            if let Some(num) = rest.strip_suffix(suffix.as_str()) {
                if !num.is_empty() && num.bytes().all(|b| b.is_ascii_digit()) {
                    if let Ok(v) = num.parse::<u32>() { max = max.max(v); }
                }
            }
        }
    }
    max + 1
}

/// ファイル名に使えない文字を置き換える (Windows)。
pub fn sanitize(name: &str) -> String {
    let bad = ['\\', '/', ':', '*', '?', '"', '<', '>', '|'];
    let s: String = name.chars().map(|c| if bad.contains(&c) || c.is_control() { '_' } else { c }).collect();
    let s = s.trim().trim_end_matches('.').to_string();
    if s.is_empty() { "_".to_string() } else { s }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(a: &[&str]) -> Vec<String> { a.iter().map(|s| s.to_string()).collect() }

    #[test]
    fn render_pads() {
        assert_eq!(render("shot_{date}_{n}", 3, 3, "20261008", "0958"), "shot_20261008_003");
        assert_eq!(render("{n}", 1234, 3, "", ""), "1234");
    }

    #[test]
    fn next_is_max_plus_one_not_count() {
        // 2 番を消してあっても、連番は詰めない (最大 + 1)
        let ex = v(&["shot_20261008_001.png", "shot_20261008_003.png", "other.txt"]);
        assert_eq!(next_number("shot_{date}_{n}", &ex, "20261008", ""), 4);
    }

    #[test]
    fn next_ignores_other_dates_and_extensions() {
        let ex = v(&["shot_20261007_009.png", "shot_20261008_002.jpg"]);
        assert_eq!(next_number("shot_{date}_{n}", &ex, "20261008", ""), 3);
        assert_eq!(next_number("shot_{date}_{n}", &[], "20261008", ""), 1);
    }

    #[test]
    fn next_with_suffix_and_no_counter() {
        let ex = v(&["a_001_end.png", "a_007_end.png", "a_009.png"]);
        assert_eq!(next_number("a_{n}_end", &ex, "", ""), 8);
        assert_eq!(next_number("fixed", &ex, "", ""), 1);
    }

    #[test]
    fn sanitize_windows() {
        assert_eq!(sanitize("a:b/c*d?.png"), "a_b_c_d_.png");
        assert_eq!(sanitize("  .. "), "_");
    }
}
