pub fn valid_key(key: &str) -> bool {
    key.len() == 32 && key.bytes().all(|b| b.is_ascii_hexdigit())
}

pub fn authorized(expected: &str, actual: &str) -> bool {
    if !valid_key(expected) || actual.len() != expected.len() {
        return false;
    }
    expected
        .bytes()
        .zip(actual.bytes())
        .fold(0u8, |d, (a, b)| d | (a ^ b))
        == 0
}

pub fn allowed_origin(origin: Option<&str>, site: Option<&str>, host: &str) -> bool {
    site != Some("cross-site") && origin.is_none_or(|o| o == format!("http://{host}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn authentication_fails_closed() {
        let key = "0123456789abcdef0123456789abcdef";
        assert!(authorized(key, key));
        assert!(!authorized("", ""));
        assert!(!authorized(key, "0123456789abcdef0123456789abcdee"));
        assert!(!authorized(key, "0123456789abcdef0123456789abcde0"));
        assert!(!allowed_origin(Some("http://evil.example"), None, "device"));
        assert!(!allowed_origin(None, Some("cross-site"), "device"));
        assert!(allowed_origin(
            Some("http://device"),
            Some("same-origin"),
            "device"
        ));
    }
}
