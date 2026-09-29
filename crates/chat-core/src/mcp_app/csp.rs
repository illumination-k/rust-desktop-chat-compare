//! Content Security Policy for views, built from `_meta.ui.csp`.

use serde_json::Value;

/// Declared origins for `key`, dropping anything that could break out of a
/// CSP source list (`;`, quotes, whitespace).
fn sources(csp: &Value, key: &str) -> Vec<String> {
    csp.get(key)
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .filter(|d| {
            !d.is_empty() && !d.contains(|c: char| c.is_whitespace() || ";,'\"".contains(c))
        })
        .map(str::to_owned)
        .collect()
}

fn directive(name: &str, base: &str, extra: &[String]) -> String {
    std::iter::once(format!("{name} {base}"))
        .chain(extra.iter().cloned())
        .collect::<Vec<_>>()
        .join(" ")
}

fn directive_or(name: &str, extra: &[String], fallback: &str) -> String {
    if extra.is_empty() {
        format!("{name} {fallback}")
    } else {
        format!("{name} {}", extra.join(" "))
    }
}

/// The spec's restrictive default plus the declared domains only.
pub fn build(csp: &Value) -> String {
    let res = sources(csp, "resourceDomains");
    [
        "default-src 'none'".to_owned(),
        directive("script-src", "'self' 'unsafe-inline'", &res),
        directive("style-src", "'self' 'unsafe-inline'", &res),
        directive("img-src", "'self' data:", &res),
        directive("font-src", "'self' data:", &res),
        directive("media-src", "'self' data:", &res),
        directive_or("connect-src", &sources(csp, "connectDomains"), "'none'"),
        directive_or("frame-src", &sources(csp, "frameDomains"), "'none'"),
        "object-src 'none'".to_owned(),
        directive_or("base-uri", &sources(csp, "baseUriDomains"), "'self'"),
    ]
    .join("; ")
}

/// Puts a CSP `<meta>` before any markup (right after the doctype), so no
/// script in the view runs before the policy applies.
pub fn inject(html: &str, policy: &str) -> String {
    let meta = format!(
        r#"<meta http-equiv="Content-Security-Policy" content="{}">"#,
        policy.replace('"', "&quot;")
    );
    let trimmed = html.trim_start();
    let at = if trimmed
        .get(..9)
        .is_some_and(|s| s.eq_ignore_ascii_case("<!doctype"))
    {
        let start = html.len() - trimmed.len();
        trimmed.find('>').map_or(0, |end| start + end + 1)
    } else {
        0
    };
    format!("{}{meta}{}", &html[..at], &html[at..])
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn default_policy_is_restrictive() {
        let csp = build(&Value::Null);
        for part in [
            "default-src 'none'",
            "connect-src 'none'",
            "frame-src 'none'",
            "object-src 'none'",
            "base-uri 'self'",
        ] {
            assert!(csp.contains(part), "{csp}");
        }
    }

    #[test]
    fn declared_domains_are_allowed_and_injection_is_dropped() {
        let csp = build(&json!({
            "connectDomains": ["https://api.example.com"],
            "resourceDomains": ["https://cdn.example.com", "https://x; script-src *"]
        }));
        assert!(csp.contains("connect-src https://api.example.com;"));
        assert!(csp.contains("script-src 'self' 'unsafe-inline' https://cdn.example.com;"));
        assert!(!csp.contains("script-src *"));
    }

    #[test]
    fn meta_goes_right_after_the_doctype() {
        assert_eq!(
            inject("<!DOCTYPE html><html>", "p"),
            r#"<!DOCTYPE html><meta http-equiv="Content-Security-Policy" content="p"><html>"#
        );
        assert_eq!(
            inject("<p>x</p>", "p"),
            r#"<meta http-equiv="Content-Security-Policy" content="p"><p>x</p>"#
        );
    }
}
