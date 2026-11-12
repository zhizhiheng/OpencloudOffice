//! `oco`: command-line front end to the OpencloudOffice core.

mod corpus;
mod inspect;
mod render;

use std::process::ExitCode;

const USAGE: &str = "\
usage: oco inspect <file.docx> [--json]
       oco corpus <dir>... [--json <report.json>] [--jobs <n>] [--layout]
       oco render <file.docx> [--out <dir>] [--fonts <dir>]... [--model word|libreoffice]

inspect  Prints every paragraph and run with its effective formatting, that
         is, after the style hierarchy and document defaults have been
         applied, followed by the markup the reader does not model yet.

corpus   Reads every Word file under the given directories and reports how
         many open, how long they take and which unmodelled markup affects
         the most documents. With --layout every document is also laid out,
         which checks that the layout engine survives real-world input.

render   Lays the document out and writes one SVG per page, plus lines.json
         with the position of every line. Fonts are taken from ./fonts and
         the system's Liberation and Carlito fonts unless --fonts is given.
         --model libreoffice follows LibreOffice where it differs from Word
         (line height from the replacement fonts' own metrics, for one),
         which is only useful for comparing against LibreOffice.";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    match args.as_slice() {
        ["inspect", path] => inspect::run(path, false),
        ["inspect", path, "--json"] | ["inspect", "--json", path] => inspect::run(path, true),
        ["corpus", rest @ ..] if !rest.is_empty() => match corpus::Options::parse(rest) {
            Some(options) => corpus::run(&options),
            None => usage(),
        },
        ["render", rest @ ..] => match render::Arguments::parse(rest) {
            Some(arguments) => render::run(&arguments),
            None => usage(),
        },
        _ => usage(),
    }
}

fn usage() -> ExitCode {
    eprintln!("{USAGE}");
    ExitCode::from(2)
}
