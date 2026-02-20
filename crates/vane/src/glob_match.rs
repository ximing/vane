/// POSIX-style glob. `path` uses `/` separators. Supports `*`, `?`, `**`, and `{a,b}`.
pub fn glob_match(pattern: &str, path: &str) -> bool {
    expand_braces(pattern)
        .into_iter()
        .any(|expanded| match_expanded(&expanded, path))
}

fn expand_braces(pattern: &str) -> Vec<String> {
    match find_brace_group(pattern) {
        None => vec![pattern.to_string()],
        Some((start, end, alts)) => {
            let prefix = &pattern[..start];
            let suffix = &pattern[end + 1..];
            let mut out = Vec::new();
            for alt in alts {
                out.extend(expand_braces(&format!("{prefix}{alt}{suffix}")));
            }
            out
        }
    }
}

fn find_brace_group(s: &str) -> Option<(usize, usize, Vec<String>)> {
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] != b'{' {
            i += 1;
            continue;
        }
        let start = i;
        let mut depth = 1;
        let mut j = i + 1;
        let mut alt_from = j;
        let mut alts = Vec::new();
        while j < bytes.len() {
            match bytes[j] {
                b'{' => depth += 1,
                b',' if depth == 1 => {
                    alts.push(s[alt_from..j].to_string());
                    alt_from = j + 1;
                }
                b'}' => {
                    depth -= 1;
                    if depth == 0 {
                        alts.push(s[alt_from..j].to_string());
                        return Some((start, j, alts));
                    }
                }
                _ => {}
            }
            j += 1;
        }
        i += 1;
    }
    None
}

fn split_segs(s: &str) -> Vec<&str> {
    s.split('/')
        .filter(|p| !p.is_empty() && *p != ".")
        .collect()
}

fn match_expanded(pattern: &str, path: &str) -> bool {
    match_segs(&split_segs(pattern), &split_segs(path))
}

fn match_segs(pat: &[&str], path: &[&str]) -> bool {
    if pat.is_empty() {
        return path.is_empty();
    }
    if pat[0] == "**" {
        if pat.len() == 1 {
            return true;
        }
        return (0..=path.len()).any(|i| match_segs(&pat[1..], &path[i..]));
    }
    if path.is_empty() {
        return pat.iter().all(|p| *p == "**");
    }
    match_component(pat[0], path[0]) && match_segs(&pat[1..], &path[1..])
}

fn match_component(pat: &str, s: &str) -> bool {
    let pat: Vec<char> = pat.chars().collect();
    let s: Vec<char> = s.chars().collect();
    let mut i = 0;
    let mut j = 0;
    let mut star: Option<(usize, usize)> = None;
    while j < s.len() {
        if i < pat.len() && pat[i] == '*' {
            star = Some((i + 1, j));
            i += 1;
        } else if i < pat.len() && (pat[i] == '?' || pat[i] == s[j]) {
            i += 1;
            j += 1;
        } else if let Some((pi, sj)) = star {
            i = pi;
            j = sj + 1;
            star = Some((pi, sj + 1));
        } else {
            return false;
        }
    }
    while i < pat.len() && pat[i] == '*' {
        i += 1;
    }
    i == pat.len()
}

#[cfg(test)]
mod tests {
    use super::glob_match;

    #[test]
    fn double_star_and_braces() {
        assert!(glob_match("**/*.md", "docs/a.md"));
        assert!(glob_match("**/node_modules/**", "app/node_modules/x/y.js"));
        assert!(glob_match("**/*.{md,txt}", "a.txt"));
        assert!(!glob_match("**/*.md", "a.rs"));
        assert!(!glob_match("**/.git/**", "docs/git-notes.md"));
    }

    #[test]
    fn double_star_matches_root_file_and_nested() {
        assert!(glob_match("**/*.md", "a.md"));
        assert!(glob_match("**/*.md", "a/b/c.md"));
        assert!(glob_match("**/.env.*", ".env.local"));
        assert!(glob_match("**/.env.*", "app/.env.production"));
        assert!(!glob_match("**/.env.*", "app/env.local"));
    }

    #[test]
    fn question_mark_and_star_stay_in_segment() {
        assert!(glob_match("docs/?.md", "docs/a.md"));
        assert!(!glob_match("docs/?.md", "docs/ab.md"));
        assert!(!glob_match("*.md", "docs/a.md"));
        assert!(glob_match("docs/*", "docs/a.md"));
        assert!(!glob_match("docs/*", "docs/sub/a.md"));
    }
}
