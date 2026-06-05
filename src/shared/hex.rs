pub fn encode(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use rstest::rstest;

    #[rstest]
    #[case::empty(&[], "")]
    #[case::single_zero(&[0x00], "00")]
    #[case::single_ff(&[0xff], "ff")]
    #[case::lowercase_padded(&[0x0a, 0xbc], "0abc")]
    #[case::multi(&[0xde, 0xad, 0xbe, 0xef], "deadbeef")]
    fn encode_cases(#[case] input: &[u8], #[case] expected: &str) {
        assert_eq!(encode(input), expected);
    }
}
