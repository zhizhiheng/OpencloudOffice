//! `oco render`: lay a document out and write each page as SVG.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use oco_docx::Docx;
use oco_layout::{svg, FontDb, Model, Options};

pub struct Arguments {
    input: PathBuf,
    out: PathBuf,
    fonts: Vec<PathBuf>,
    model: Model,
}

impl Arguments {
    pub fn parse(args: &[&str]) -> Option<Self> {
        let mut parsed =
            Arguments { input: PathBuf::new(), out: PathBuf::from("out"), fonts: Vec::new(), model: Model::Word };
        let mut input = None;
        let mut args = args.iter();
        while let Some(arg) = args.next() {
            match *arg {
                "--out" => parsed.out = PathBuf::from(args.next()?),
                "--fonts" => parsed.fonts.push(PathBuf::from(args.next()?)),
                "--model" => {
                    parsed.model = match *args.next()? {
                        "word" => Model::Word,
                        "libreoffice" => Model::LibreOffice,
                        _ => return None,
                    };
                }
                flag if flag.starts_with("--") => return None,
                path => input = Some(PathBuf::from(path)),
            }
        }
        parsed.input = input?;
        Some(parsed)
    }
}

/// Where fonts are looked for when none are named: the project's `fonts`
/// directory, and the metric-compatible Latin fonts most systems carry.
pub const DEFAULT_FONT_DIRS: &[&str] = &[
    "fonts",
    "/usr/share/fonts/truetype/liberation",
    "/usr/share/fonts/truetype/crosextra",
    "/usr/share/fonts/truetype/dejavu",
];

pub fn run(arguments: &Arguments) -> ExitCode {
    let docx = match Docx::open(&arguments.input) {
        Ok(docx) => docx,
        Err(error) => {
            eprintln!("oco: {}: {error}", arguments.input.display());
            return ExitCode::FAILURE;
        }
    };
    let mut fonts = FontDb::default();
    if arguments.fonts.is_empty() {
        DEFAULT_FONT_DIRS.iter().for_each(|dir| fonts.load_dir(Path::new(dir)));
    } else {
        arguments.fonts.iter().for_each(|dir| fonts.load_dir(dir));
    }
    if fonts.is_empty() {
        eprintln!("oco: no fonts found; run tools/fonts/fetch.sh or pass --fonts <dir>");
        return ExitCode::FAILURE;
    }

    let pages = oco_layout::layout(&docx, &fonts, &Options { model: arguments.model }).pages;
    if let Err(error) = write(&arguments.out, &pages, &fonts) {
        eprintln!("oco: {}: {error}", arguments.out.display());
        return ExitCode::FAILURE;
    }
    let lines: usize = pages.iter().map(|page| page.lines.len()).sum();
    println!("{}: {} pages, {lines} lines -> {}", arguments.input.display(), pages.len(), arguments.out.display());
    ExitCode::SUCCESS
}

fn write(out: &Path, pages: &[oco_layout::Page], fonts: &FontDb) -> std::io::Result<()> {
    fs::create_dir_all(out)?;
    for (index, page) in pages.iter().enumerate() {
        fs::write(out.join(format!("page-{}.svg", index + 1)), svg::render(page, fonts))?;
    }
    // Line positions in the shape tools/zh/reference.py writes for LibreOffice.
    let summary: Vec<_> = pages
        .iter()
        .map(|page| serde_json::json!({ "width": page.width, "height": page.height, "lines": page.lines }))
        .collect();
    fs::write(out.join("lines.json"), serde_json::to_string_pretty(&summary).expect("plain data serialises"))
}
