use std::io;

use crate::shared::command;

pub(super) fn open_url(url: &str) -> io::Result<()> {
    let opener = if cfg!(target_os = "macos") {
        "open"
    } else {
        "xdg-open"
    };

    command::new(opener).arg(url).spawn().map(|_| ())
}
