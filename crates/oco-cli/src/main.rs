//! `oco`: command-line front end to the OpencloudOffice core.

mod corpus;
mod inspect;
mod render;
mod score;

use std::process::ExitCode;

const USAGE: &str = "\
usage: oco inspect <file> [--json]
       oco corpus <dir>... [--json <report.json>] [--jobs <n>] [--layout]
       oco render <file> [--out <dir>] [--fonts <dir>]... [--model word|libreoffice]
       oco convert <file> <out.docx>
       oco score <dir or file>... [--json <report.json>] [--jobs <n>] [--explain]

A <file> is a Word document (.docx, or the older .doc) or Markdown (.md).

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
         which is only useful for comparing against LibreOffice.

convert  Writes a .doc or a Markdown file as a .docx: the document form
         that every kind of file is shown from. Word opens the result.

score    Lays out every document that Word saved and compares the result
         with what Word recorded in the file: its page count and where
         each page began. This measures layout fidelity against Word
         without running Word. --explain lists, for each document, every
         page start Word recorded beside the nearest of ours.";

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
        ["convert", input, output] => convert(input, output),
        ["score", rest @ ..] => match score::Arguments::parse(rest) {
            Some(arguments) => score::run(&arguments),
            None => usage(),
        },
        ["render", rest @ ..] => match render::Arguments::parse(rest) {
            Some(arguments) => render::run(&arguments),
            None => usage(),
        },
        _ => usage(),
    }
}

fn convert(input: &str, output: &str) -> ExitCode {
    let path = std::path::Path::new(input);
    let name = path.file_name().map(|name| name.to_string_lossy().into_owned());
    // Images of a Markdown file are looked for beside it, and nowhere above it.
    let beside = |address: &str| {
        let relative = std::path::Path::new(address);
        let inside = relative.components().all(|part| matches!(part, std::path::Component::Normal(_)));
        inside.then(|| std::fs::read(path.parent()?.join(relative)).ok()).flatten()
    };
    let written = std::fs::read(path).map_err(oco_formats::Error::Io).and_then(|bytes| {
        let package = oco_formats::converted(&bytes, name.as_deref(), &beside)?;
        std::fs::write(output, package.as_deref().unwrap_or(&bytes)).map_err(oco_formats::Error::Io)
    });
    match written {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("oco: {input}: {error}");
            ExitCode::FAILURE
        }
    }
}

fn usage() -> ExitCode {
    eprintln!("{USAGE}");
    ExitCode::from(2)
}
