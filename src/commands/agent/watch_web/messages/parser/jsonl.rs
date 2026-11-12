use std::io::{self, BufRead};

#[derive(Clone, Default)]
pub(super) struct JsonlLineBuffer {
    pending: Vec<u8>,
}

impl JsonlLineBuffer {
    pub(super) fn feed(&mut self, bytes: &[u8], mut visit: impl FnMut(&[u8])) {
        self.pending.extend_from_slice(bytes);
        let mut start = 0;
        while let Some(relative_end) = self.pending[start..].iter().position(|byte| *byte == b'\n')
        {
            let end = start + relative_end + 1;
            visit(&self.pending[start..end]);
            start = end;
        }
        if start != 0 {
            self.pending.drain(..start);
        }
    }

    pub(super) fn read_from(
        &mut self,
        reader: &mut impl BufRead,
        mut visit: impl FnMut(&[u8]),
    ) -> io::Result<()> {
        let mut line = Vec::new();
        loop {
            line.clear();
            if reader.read_until(b'\n', &mut line)? == 0 {
                return Ok(());
            }
            self.feed(&line, &mut visit);
        }
    }

    pub(super) fn pending(&self) -> &[u8] {
        &self.pending
    }
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use indoc::indoc;

    use super::JsonlLineBuffer;

    #[test]
    fn jsonl_line_buffer_carries_partial_lines_across_feeds() {
        let mut lines = JsonlLineBuffer::default();
        let mut parsed = Vec::new();
        lines.feed(b"{\"type\":\"user\"", |line| parsed.push(line.to_vec()));
        lines.feed(b",\"text\":\"fixture\"}\n", |line| {
            parsed.push(line.to_vec());
        });
        lines.feed(b"{\"type\":\"assistant\"}\n", |line| {
            parsed.push(line.to_vec());
        });

        assert_eq!(
            (parsed, lines.pending(),),
            (
                vec![
                    b"{\"type\":\"user\",\"text\":\"fixture\"}\n".to_vec(),
                    b"{\"type\":\"assistant\"}\n".to_vec(),
                ],
                b"" as &[u8],
            ),
        );
    }

    #[test]
    fn jsonl_line_buffer_reads_complete_lines_and_keeps_the_unterminated_tail() {
        let transcript = indoc! {r#"
            {"type":"user"}
            {"type":"assistant"}
            {"type":"pending"}
        "#};
        let transcript = transcript.trim_end_matches('\n').as_bytes();
        let mut lines = JsonlLineBuffer::default();
        let mut parsed = Vec::new();
        lines
            .read_from(&mut Cursor::new(transcript), |line| {
                parsed.push(line.to_vec())
            })
            .expect("transcript should be read");

        assert_eq!(
            (parsed, lines.pending(),),
            (
                vec![
                    b"{\"type\":\"user\"}\n".to_vec(),
                    b"{\"type\":\"assistant\"}\n".to_vec(),
                ],
                b"{\"type\":\"pending\"}" as &[u8],
            ),
        );
    }
}
