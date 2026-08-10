[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [string]$OutputDirectory,
    [switch]$Validate
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

Add-Type -AssemblyName System.IO.Compression
Add-Type -AssemblyName System.IO.Compression.FileSystem

$OutputDirectory = [IO.Path]::GetFullPath($OutputDirectory)
New-Item -ItemType Directory -Force -Path $OutputDirectory | Out-Null

function Write-ZipPackage {
    param(
        [Parameter(Mandatory = $true)]
        [string]$Path,
        [Parameter(Mandatory = $true)]
        [hashtable]$Entries
    )

    if (Test-Path -LiteralPath $Path) {
        Remove-Item -LiteralPath $Path -Force
    }
    $stream = [IO.File]::Open($Path, [IO.FileMode]::CreateNew, [IO.FileAccess]::Write)
    $archive = [IO.Compression.ZipArchive]::new(
        $stream,
        [IO.Compression.ZipArchiveMode]::Create,
        $false
    )
    try {
        foreach ($entry in $Entries.GetEnumerator()) {
            $zipEntry = $archive.CreateEntry($entry.Key)
            $entryStream = $zipEntry.Open()
            try {
                if ($entry.Value -is [byte[]]) {
                    $entryStream.Write($entry.Value, 0, $entry.Value.Length)
                } else {
                    $bytes = [Text.UTF8Encoding]::new($false).GetBytes([string]$entry.Value)
                    $entryStream.Write($bytes, 0, $bytes.Length)
                }
            } finally {
                $entryStream.Dispose()
            }
        }
    } finally {
        $archive.Dispose()
        $stream.Dispose()
    }
}

function New-PdfBytes {
    $nl = [char]10
    $content = "BT" + $nl +
        "/F1 18 Tf" + $nl +
        "72 720 Td" + $nl +
        "(LunaScope deterministic release canary lecture fixture.) Tj" + $nl +
        "/F1 12 Tf" + $nl +
        "0 -28 Td" + $nl +
        "(This fixture contains stable source-grounded calculus notes for attachment extraction.) Tj" + $nl +
        "ET"
    $contentObject = "<< /Length $([Text.Encoding]::ASCII.GetByteCount($content)) >>" + $nl + "stream" + $nl + $content + $nl + "endstream"
    $objects = @(
        "<< /Type /Catalog /Pages 2 0 R >>",
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>",
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << /Font << /F1 5 0 R >> >> /Contents 4 0 R >>",
        $contentObject,
        "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>"
    )
    $bytes = [Collections.Generic.List[byte]]::new()
    $ascii = [Text.Encoding]::ASCII
    $add = {
        param([string]$Value)
        $bytes.AddRange($ascii.GetBytes($Value))
    }
    & $add ("%PDF-1.4" + $nl)
    $offsets = @(0)
    for ($index = 0; $index -lt $objects.Count; $index++) {
        $offsets += $bytes.Count
        & $add (($index + 1).ToString() + " 0 obj" + $nl + $objects[$index] + $nl + "endobj" + $nl)
    }
    $xrefOffset = $bytes.Count
    & $add ("xref" + $nl + "0 " + ($objects.Count + 1).ToString() + $nl)
    & $add ("0000000000 65535 f " + $nl)
    foreach ($offset in $offsets[1..$objects.Count]) {
        & $add (("{0:D10} 00000 n " -f $offset) + $nl)
    }
    & $add ("trailer" + $nl + "<< /Size " + ($objects.Count + 1).ToString() + " /Root 1 0 R >>" + $nl + "startxref" + $nl + $xrefOffset.ToString() + $nl + "%%EOF" + $nl)
    return $bytes.ToArray()
}

$png = [Convert]::FromBase64String(
    "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNk+A8AAQUBAScY42YAAAAASUVORK5CYII="
)

$docxContentTypes = @'
<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">
  <Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/>
  <Default Extension="xml" ContentType="application/xml"/>
  <Default Extension="png" ContentType="image/png"/>
  <Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/>
</Types>
'@
$docxRootRels = @'
<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
  <Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/>
</Relationships>
'@
$docxDocument = @'
<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
  <w:body>
    <w:p><w:r><w:t>LunaScope deterministic release canary lecture fixture.</w:t></w:r></w:p>
    <w:p><w:r><w:t>This document provides stable source-grounded calculus notes for the UltraNote attachment canary.</w:t></w:r></w:p>
    <w:sectPr><w:pgSz w:w="12240" w:h="15840"/><w:pgMar w:top="1440" w:right="1440" w:bottom="1440" w:left="1440"/></w:sectPr>
  </w:body>
</w:document>
'@
$docxRels = @'
<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
  <Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/image" Target="media/fixture.png"/>
</Relationships>
'@
Write-ZipPackage (Join-Path $OutputDirectory "lecture.docx") @{
    "[Content_Types].xml" = $docxContentTypes
    "_rels/.rels" = $docxRootRels
    "word/document.xml" = $docxDocument
    "word/_rels/document.xml.rels" = $docxRels
    "word/media/fixture.png" = $png
}

$xlsxContentTypes = @'
<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">
  <Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/>
  <Default Extension="xml" ContentType="application/xml"/>
  <Default Extension="png" ContentType="image/png"/>
  <Override PartName="/xl/workbook.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml"/>
  <Override PartName="/xl/worksheets/sheet1.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"/>
</Types>
'@
$xlsxRootRels = @'
<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
  <Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="xl/workbook.xml"/>
</Relationships>
'@
$xlsxWorkbook = @'
<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships">
  <sheets><sheet name="Lecture" sheetId="1" r:id="rId1"/></sheets>
</workbook>
'@
$xlsxRels = @'
<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
  <Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet1.xml"/>
  <Relationship Id="rId2" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/image" Target="media/fixture.png"/>
</Relationships>
'@
$xlsxSheet = @'
<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main">
  <sheetData>
    <row r="1"><c r="A1" t="inlineStr"><is><t>LunaScope deterministic release canary lecture fixture</t></is></c></row>
    <row r="2"><c r="A2" t="inlineStr"><is><t>Stable source-grounded calculus notes for the UltraNote attachment canary.</t></is></c></row>
  </sheetData>
</worksheet>
'@
Write-ZipPackage (Join-Path $OutputDirectory "lecture.xlsx") @{
    "[Content_Types].xml" = $xlsxContentTypes
    "_rels/.rels" = $xlsxRootRels
    "xl/workbook.xml" = $xlsxWorkbook
    "xl/_rels/workbook.xml.rels" = $xlsxRels
    "xl/worksheets/sheet1.xml" = $xlsxSheet
    "xl/media/fixture.png" = $png
}

$pptxContentTypes = @'
<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">
  <Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/>
  <Default Extension="xml" ContentType="application/xml"/>
  <Default Extension="png" ContentType="image/png"/>
  <Override PartName="/ppt/presentation.xml" ContentType="application/vnd.openxmlformats-officedocument.presentationml.presentation.main+xml"/>
  <Override PartName="/ppt/slides/slide1.xml" ContentType="application/vnd.openxmlformats-officedocument.presentationml.slide+xml"/>
</Types>
'@
$pptxRootRels = @'
<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
  <Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="ppt/presentation.xml"/>
</Relationships>
'@
$pptxPresentation = @'
<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<p:presentation xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships" xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main">
  <p:sldMasterIdLst/><p:notesMasterIdLst/><p:handoutMasterIdLst/>
  <p:sldIdLst><p:sldId id="256" r:id="rId1"/></p:sldIdLst>
  <p:sldSz cx="12192000" cy="6858000" type="screen4x3"/><p:notesSz cx="6858000" cy="9144000"/>
</p:presentation>
'@
$pptxPresentationRels = @'
<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
  <Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/slide" Target="slides/slide1.xml"/>
</Relationships>
'@
$pptxSlide = @'
<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<p:sld xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships" xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main">
  <p:cSld><p:spTree>
    <p:nvGrpSpPr><p:cNvPr id="1" name=""/><p:cNvGrpSpPr/><p:nvPr/></p:nvGrpSpPr>
    <p:grpSpPr/>
    <p:sp><p:nvSpPr><p:cNvPr id="2" name="Title"/><p:cNvSpPr txBox="1"/><p:nvPr/></p:nvSpPr><p:spPr/><p:txBody><a:bodyPr/><a:lstStyle/><a:p><a:r><a:rPr lang="en-US" sz="1800"/><a:t>LunaScope deterministic release canary lecture fixture</a:t></a:r></a:p><a:p><a:r><a:rPr lang="en-US" sz="1200"/><a:t>Stable source-grounded calculus notes for the UltraNote attachment canary.</a:t></a:r></a:p></p:txBody></p:sp>
  </p:spTree></p:cSld><p:clrMapOvr><a:masterClrMapping/></p:clrMapOvr>
</p:sld>
'@
$pptxSlideRels = @'
<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
  <Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/image" Target="../media/fixture.png"/>
</Relationships>
'@
Write-ZipPackage (Join-Path $OutputDirectory "lecture.pptx") @{
    "[Content_Types].xml" = $pptxContentTypes
    "_rels/.rels" = $pptxRootRels
    "ppt/presentation.xml" = $pptxPresentation
    "ppt/_rels/presentation.xml.rels" = $pptxPresentationRels
    "ppt/slides/slide1.xml" = $pptxSlide
    "ppt/slides/_rels/slide1.xml.rels" = $pptxSlideRels
    "ppt/media/fixture.png" = $png
}

[IO.File]::WriteAllBytes((Join-Path $OutputDirectory "lecture.pdf"), (New-PdfBytes))
[IO.File]::WriteAllBytes((Join-Path $OutputDirectory "formula.png"), $png)

if ($Validate) {
    $required = @("lecture.pdf", "lecture.docx", "lecture.xlsx", "lecture.pptx", "formula.png")
    foreach ($name in $required) {
        $path = Join-Path $OutputDirectory $name
        if (-not (Test-Path -LiteralPath $path -PathType Leaf)) {
            throw "Missing release canary fixture: $name"
        }
        if ((Get-Item -LiteralPath $path).Length -le 0) {
            throw "Empty release canary fixture: $name"
        }
    }
    foreach ($package in @("lecture.docx", "lecture.xlsx", "lecture.pptx")) {
        $path = Join-Path $OutputDirectory $package
        $stream = [IO.File]::OpenRead($path)
        $archive = [IO.Compression.ZipArchive]::new($stream, [IO.Compression.ZipArchiveMode]::Read, $false)
        try {
            if ($archive.Entries.Count -lt 4) {
                throw "Office fixture is not a complete OOXML package: $package"
            }
        } finally {
            $archive.Dispose()
            $stream.Dispose()
        }
    }
    Write-Output "release_canary_fixtures=valid path=$OutputDirectory"
}
