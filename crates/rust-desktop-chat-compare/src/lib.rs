/// Adds two numbers.
///
/// # Examples
///
/// ```
/// use rust_desktop_chat_compare::add;
///
/// assert_eq!(add(2, 2), 4);
/// ```
pub fn add(left: u64, right: u64) -> u64 {
    tracing::debug!(left, right, "adding");
    left + right
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn it_works() {
        let result = add(2, 2);
        assert_eq!(result, 4);
    }
}
