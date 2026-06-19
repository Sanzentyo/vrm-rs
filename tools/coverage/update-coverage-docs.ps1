param(
    [string]$Command = "cargo llvm-cov --workspace --all-features --summary-only --fail-under-lines 70",
    [string]$TestingDoc = "docs/testing.md",
    [string]$ProgressDoc = "docs/progress.md",
    [string]$SummaryJsonPath = "",
    [string]$Date = (Get-Date -Format "yyyy-MM-dd"),
    [switch]$Apply
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

function Get-SummaryJson {
    param([string]$CommandText, [string]$JsonPath)

    if ($JsonPath -and (Test-Path $JsonPath)) {
        return (Get-Content -Raw -LiteralPath $JsonPath | ConvertFrom-Json)
    }

    $workDir = Split-Path -Parent $CommandText
    if (-not $workDir) { $workDir = Get-Location }
    if (-not (Test-Path $workDir)) { $workDir = Get-Location }
    $generatedPath = Join-Path $workDir "target/coverage-summary-from-spark.json"

    if (Test-Path $generatedPath) {
        Remove-Item $generatedPath -Force
    }

    $runCommand = $CommandText
    if ($runCommand -notmatch "(^|\s)--json(\s|$)") {
        $runCommand += " --json"
    }
    if ($runCommand -notmatch "(^|\s)--summary-only(\s|$)") {
        $runCommand += " --summary-only"
    }
    if ($runCommand -notmatch "(^|\s)--output-path\s") {
        $runCommand += " --output-path `"$generatedPath`""
    }

    Write-Host "Running coverage command:"
    Write-Host "  $runCommand"
    & cmd /c $runCommand
    if (-not (Test-Path $generatedPath)) {
        throw "Coverage JSON was not generated. Check command output and rerun with --output-path."
    }

    return (Get-Content -Raw -LiteralPath $generatedPath | ConvertFrom-Json)
}

function Format-Percent {
    param([double]$Value)
    return ("{0:F2}" -f [math]::Round($Value, 2))
}

function Get-CrateSummaryMap {
    param([object]$CoverageData)

    $rawMap = @{}
    foreach ($file in $CoverageData.data[0].files) {
        $name = ""
        if ($file.filename -match '(?:[\\\/])crates(?:[\\\/])([^\\\/]+)(?:[\\\/])src(?:[\\\/]).+\.rs$') {
            $name = $Matches[1]
        } elseif ($file.filename -match '(?:[\\\/])src(?:[\\\/])lib\.rs$') {
            $name = "src/lib.rs"
        } else {
            continue
        }

        if (-not $rawMap.ContainsKey($name)) {
            $rawMap[$name] = @{
                RegionCount = 0.0
                RegionCovered = 0.0
                LineCount = 0.0
                LineCovered = 0.0
            }
        }
        $entry = $rawMap[$name]
        $entry.RegionCount += [double]$file.summary.regions.count
        $entry.RegionCovered += [double]$file.summary.regions.covered
        $entry.LineCount += [double]$file.summary.lines.count
        $entry.LineCovered += [double]$file.summary.lines.covered
    }

    $map = @{}
    foreach ($name in $rawMap.Keys) {
        $entry = $rawMap[$name]
        $map[$name] = @{
            Regions = if ($entry.RegionCount -gt 0) { ($entry.RegionCovered * 100.0) / $entry.RegionCount } else { 0.0 }
            Lines = if ($entry.LineCount -gt 0) { ($entry.LineCovered * 100.0) / $entry.LineCount } else { 0.0 }
        }
    }

    $totals = @{
        Regions = [double]$CoverageData.data[0].totals.regions.percent
        Lines = [double]$CoverageData.data[0].totals.lines.percent
    }

    return @{ Crates = $map; Totals = $totals }
}

function Build-TestingSection {
    param([string]$DateText, [string]$CommandText, [hashtable]$SummaryMap)

    $md = [char]96
    $rows = @(
        @{ Name = "Workspace total"; Key = "workspace" },
        @{ Name = "vrm-adapter-ash"; Key = "vrm-adapter-ash" },
        @{ Name = "vrm-adapter-bevy"; Key = "vrm-adapter-bevy" },
        @{ Name = "vrm-adapter-wgpu"; Key = "vrm-adapter-wgpu" },
        @{ Name = "vrm-adapter"; Key = "vrm-adapter" },
        @{ Name = "vrm-core"; Key = "vrm-core" },
        @{ Name = "vrm-diagnostics"; Key = "vrm-diagnostics" },
        @{ Name = "vrm-io"; Key = "vrm-io" },
        @{ Name = "vrm-osc"; Key = "vrm-osc" },
        @{ Name = "vrm-protocol"; Key = "vrm-protocol" },
        @{ Name = "vrm-runtime"; Key = "vrm-runtime" },
        @{ Name = "vrm-sans-io"; Key = "vrm-sans-io" },
        @{ Name = "vrm-vmc"; Key = "vrm-vmc" },
        @{ Name = "src/lib.rs"; Key = "src/lib.rs" }
    )

    $output = @()
    $output += "## Current Coverage Snapshot"
    $output += ""
    $output += "Measured locally on $DateText with:"
    $output += ""
    $output += '```powershell'
    $output += $CommandText
    $output += '```'
    $output += ""
    $output += '| Scope | Region coverage | Line coverage |'
    $output += '| --- | ---: | ---: |'

    foreach ($row in $rows) {
        if ($row.Key -eq "workspace") {
            $region = Format-Percent -Value $SummaryMap.Totals.Regions
            $line = Format-Percent -Value $SummaryMap.Totals.Lines
        } else {
            if (-not $SummaryMap.Crates.ContainsKey($row.Key)) {
                continue
            }
            $crate = $SummaryMap.Crates[$row.Key]
            $region = Format-Percent -Value $crate.Regions
            $line = Format-Percent -Value $crate.Lines
        }
        if ($row.Key -eq "src/lib.rs") {
            $scope = "$md" + "facade " + $row.Name + "$md"
        } elseif ($row.Key -eq "workspace") {
            $scope = $row.Name
        } else {
            $scope = "$md" + $row.Name + "$md"
        }
        $output += "| $scope | $($region)% | $($line)% |"
    }

    return $output
}

function Update-TestingDoc {
    param([string]$Path, [string]$DateText, [string]$CommandText, [hashtable]$SummaryMap)

    $lines = Get-Content -LiteralPath $Path
    $start = ($lines | Select-String -Pattern "^## Current Coverage Snapshot").LineNumber
    if (-not $start) { throw "Could not find '## Current Coverage Snapshot' in $Path" }

    $startIdx = $start - 1
    $endIdx = $startIdx + 1
    while ($endIdx -lt $lines.Length -and -not ($lines[$endIdx] -match "^## ")) {
        $endIdx++
    }

    $oldSection = $lines[$startIdx..($endIdx - 1)]
    $lastTableLine = -1
    for ($i = 0; $i -lt $oldSection.Length; $i++) {
        if ($oldSection[$i] -match "^\|") {
            $lastTableLine = $i
        }
    }

    $preservedTail = @()
    if ($lastTableLine -ge 0 -and ($lastTableLine + 1) -lt $oldSection.Length) {
        $preservedTail = $oldSection[($lastTableLine + 1)..($oldSection.Length - 1)]
        while ($preservedTail.Length -gt 0 -and $preservedTail[-1] -eq "") {
            if ($preservedTail.Length -eq 1) {
                $preservedTail = @()
            } else {
                $preservedTail = $preservedTail[0..($preservedTail.Length - 2)]
            }
        }
    }

    $replacement = Build-TestingSection -DateText $DateText -CommandText $CommandText -SummaryMap $SummaryMap
    $newLines = @()
    $newLines += $lines[0..$startIdx]
    $newLines += ""
    if ($replacement.Length -gt 2) {
        $newLines += $replacement[2..($replacement.Length - 1)] # remove heading to keep original heading
    }
    if ($preservedTail.Length -gt 0) {
        $newLines += $preservedTail
    }
    if ($endIdx -lt $lines.Length) {
        if ($newLines.Length -gt 0 -and $newLines[-1] -ne "") {
            $newLines += ""
        }
        $newLines += $lines[$endIdx..($lines.Length - 1)]
    }

    return $newLines
}

function Build-ProgressLine {
    param([hashtable]$SummaryMap, [string]$DateText)

    $md = [char]96
    $defaultCrate = "vrm-adapter-bevy"
    $crate = $defaultCrate

    if ($SummaryMap.Crates.ContainsKey($defaultCrate)) {
        $crateLine = Format-Percent -Value $SummaryMap.Crates[$defaultCrate].Lines
    } else {
        $crateLine = Format-Percent -Value $SummaryMap.Totals.Lines
    }

    $workspace = Format-Percent -Value $SummaryMap.Totals.Lines
    return "- Re-measured coverage after workspace coverage refresh on ${DateText}: workspace line coverage is ${workspace}%, and ${md}${crate}${md} line coverage is ${crateLine}%."
}

function Update-ProgressDoc {
    param([string]$Path, [string]$LineText)

    $lines = Get-Content -LiteralPath $Path
    $lineNumbers = @()
    for ($i = 0; $i -lt $lines.Length; $i++) {
        if ($lines[$i] -match "^- Re-measured coverage .*workspace line coverage is .*%.*line coverage is .*%") {
            $lineNumbers += $i
        }
    }

    if ($lineNumbers.Count -eq 0) {
        throw "Could not find a 'Re-measured coverage ...' line in $Path"
    }

    $idx = $lineNumbers[-1]
    $lines[$idx] = $LineText
    return $lines
}

$coverage = Get-SummaryJson -CommandText $Command -JsonPath $SummaryJsonPath
$summaryMap = Get-CrateSummaryMap -CoverageData $coverage

$testingSection = Build-TestingSection -DateText $Date -CommandText $Command -SummaryMap $summaryMap
Write-Host ""
Write-Host "Generated testing snapshot block:"
$testingSection | ForEach-Object { Write-Host $_ }

$progressLine = Build-ProgressLine -SummaryMap $summaryMap -DateText $Date
Write-Host ""
Write-Host "Generated progress replacement line:"
Write-Host $progressLine

if (-not $Apply) {
    Write-Host ""
    Write-Host "Dry run only. Use -Apply to write files."
    return
}

Set-Content -LiteralPath $TestingDoc -Value (Update-TestingDoc -Path $TestingDoc -DateText $Date -CommandText $Command -SummaryMap $summaryMap)
Write-Host "Updated: $TestingDoc"

Set-Content -LiteralPath $ProgressDoc -Value (Update-ProgressDoc -Path $ProgressDoc -LineText $progressLine)
Write-Host "Updated: $ProgressDoc"
