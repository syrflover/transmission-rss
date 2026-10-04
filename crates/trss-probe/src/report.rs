//! The plain-text report: one line per fact, written as it happens so that a
//! probe the memory limit kills still shows how far it got.

use std::io::Write;

#[derive(Default)]
pub struct Report {
    checked: usize,
    failed: usize,
}

fn line(text: &str) {
    let mut out = std::io::stdout().lock();
    let _ = writeln!(out, "{text}");
    let _ = out.flush();
}

impl Report {
    pub fn section(&mut self, title: &str) {
        line(&format!("\n== {title}"));
    }

    /// A fact that is not a pass or a fail.
    pub fn info(&mut self, text: impl AsRef<str>) {
        line(&format!("[info] {}", text.as_ref()));
    }

    /// A check with an expected result.
    pub fn check(&mut self, ok: bool, name: &str, detail: impl AsRef<str>) {
        self.checked += 1;
        if !ok {
            self.failed += 1;
        }
        let mark = if ok { "[ok]  " } else { "[FAIL]" };
        line(&format!("{mark} {name}: {}", detail.as_ref()));
    }

    pub fn failed(&self) -> usize {
        self.failed
    }

    pub fn summary(&self) {
        line(&format!(
            "\n== Summary\n{} checks, {} failed",
            self.checked, self.failed
        ));
    }
}
