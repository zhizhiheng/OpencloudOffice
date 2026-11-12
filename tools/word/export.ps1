<#
.SYNOPSIS
  Has Word itself lay out a folder of documents, as the reference for
  tools/word/compare.py: one PDF per document, plus a manifest that says
  which Word made them and what Word counted in each.

.DESCRIPTION
  Run on a Windows machine with Word installed:

      powershell -ExecutionPolicy Bypass -File tools\word\export.ps1
      powershell -ExecutionPolicy Bypass -File tools\word\export.ps1 -In some\folder -Out some\folder\word

  The output goes to the `word` subfolder of the input by default. Commit it
  (for our own test documents) or copy it next to the documents on the machine
  that runs the comparison.

  Word's layout depends on more than the document: the fonts installed, the
  default printer, Word's build. The manifest records them, so that two sets
  of PDFs can be told apart. Fonts a document names but the machine lacks are
  substituted by Word without notice; install the fonts the documents use
  (for testdata\zh: the Chinese fonts that ship with Windows are enough).

  NOTE: this script was written without access to Windows or Word and has not
  been run. It uses only long-standing calls of Word's automation interface;
  if one fails, the error names the line.
#>
param(
    [string]$In = "testdata\zh",
    [string]$Out = ""
)

$ErrorActionPreference = "Stop"
$In = (Resolve-Path $In).Path
if (-not $Out) { $Out = Join-Path $In "word" }
New-Item -ItemType Directory -Force -Path $Out | Out-Null
$Out = (Resolve-Path $Out).Path

# Word's enumerations, by value.
$wdAlertsNone = 0
$wdDoNotSaveChanges = 0
$wdExportFormatPDF = 17
$wdExportOptimizeForPrint = 0
$wdExportAllDocument = 0
$wdExportDocumentContent = 0      # the document without revision marks or comments
$wdExportCreateNoBookmarks = 0
$wdStatisticWords = 0
$wdStatisticLines = 1
$wdStatisticPages = 2
$wdStatisticCharacters = 3
$wdStatisticParagraphs = 4
$wdStatisticCharactersWithSpaces = 5

$files = Get-ChildItem -Path $In -Recurse -File |
    Where-Object { ($_.Extension -in ".docx", ".docm") -and -not $_.Name.StartsWith("~$") -and -not $_.FullName.StartsWith($Out) } |
    Sort-Object FullName
if (-not $files) { throw "no .docx files under $In" }

$word = New-Object -ComObject Word.Application
$word.Visible = $false
$word.DisplayAlerts = $wdAlertsNone
$documents = [ordered]@{}
try {
    foreach ($file in $files) {
        $relative = $file.FullName.Substring($In.Length).TrimStart("\")
        $pdf = Join-Path $Out ([IO.Path]::ChangeExtension($relative, ".pdf"))
        New-Item -ItemType Directory -Force -Path (Split-Path $pdf) | Out-Null
        Write-Host $relative

        # FileName, ConfirmConversions, ReadOnly, AddToRecentFiles
        $doc = $word.Documents.Open($file.FullName, $false, $true, $false)
        try {
            $doc.Repaginate()
            $documents[$relative.Replace("\", "/")] = [ordered]@{
                pages                = $doc.ComputeStatistics($wdStatisticPages)
                lines                = $doc.ComputeStatistics($wdStatisticLines)
                paragraphs           = $doc.ComputeStatistics($wdStatisticParagraphs)
                words                = $doc.ComputeStatistics($wdStatisticWords)
                characters           = $doc.ComputeStatistics($wdStatisticCharacters)
                charactersWithSpaces = $doc.ComputeStatistics($wdStatisticCharactersWithSpaces)
                compatibilityMode    = $doc.CompatibilityMode
            }
            # OutputFileName, ExportFormat, OpenAfterExport, OptimizeFor, Range, From, To, Item,
            # IncludeDocProps, KeepIRM, CreateBookmarks, DocStructureTags, BitmapMissingFonts, UseISO19005_1.
            # BitmapMissingFonts is off so that text in a font that may not be embedded stays text.
            $doc.ExportAsFixedFormat($pdf, $wdExportFormatPDF, $false, $wdExportOptimizeForPrint,
                $wdExportAllDocument, 1, 1, $wdExportDocumentContent, $false, $false,
                $wdExportCreateNoBookmarks, $true, $false, $false)
        }
        finally {
            $doc.Close($wdDoNotSaveChanges)
        }
    }

    $os = Get-CimInstance Win32_OperatingSystem
    $manifest = [ordered]@{
        made      = (Get-Date).ToString("o")
        word      = "$($word.Version) build $($word.Build)"
        os        = "$($os.Caption) $($os.Version)"
        printer   = $word.ActivePrinter
        export    = "ExportAsFixedFormat PDF, optimised for print, without markup, fields not updated"
        documents = $documents
    }
    $json = $manifest | ConvertTo-Json -Depth 5
    [IO.File]::WriteAllText((Join-Path $Out "manifest.json"), $json, (New-Object Text.UTF8Encoding $false))
    Write-Host "$($documents.Count) documents -> $Out"
}
finally {
    $word.Quit()
    [void][Runtime.InteropServices.Marshal]::ReleaseComObject($word)
}
