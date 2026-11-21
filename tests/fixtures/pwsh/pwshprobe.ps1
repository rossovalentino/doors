# PWSH-A probe (P18 M1): PowerShell 7 itself, unmodified, running a script.
# Each step prints "PWSH-A <n> <name> ok <detail>" or "... FAILED <reason>";
# the last line is "PWSH-A PASSED" or "PWSH-A FAILED". Everything printed is
# deterministic except step 1's version, printed for the record.
$ErrorActionPreference = 'Stop'
$script:failures = 0
function Step([int]$n, [string]$name, [scriptblock]$body) {
    try {
        $detail = & $body
        "PWSH-A $n $name ok $detail"
    } catch {
        $script:failures++
        $e = $_.Exception
        $chain = @()
        while ($e) { $chain += "$($e.GetType().Name): $($e.Message)"; $e = $e.InnerException }
        "PWSH-A $n $name FAILED $($chain -join ' <- ')"
        "PWSH-A $n at $($_.ScriptStackTrace -replace '\r?\n', ' | ')"
    }
}

Step 1 'version' { "$($PSVersionTable.PSVersion.Major).$($PSVersionTable.PSVersion.Minor) $($PSVersionTable.PSEdition)" }
Step 2 'pipeline' {
    $sum = 1..20 | Where-Object { $_ % 2 } | ForEach-Object { $_ * $_ } | Measure-Object -Sum
    $sum.Sum
}
Step 3 'objects' {
    $o = [pscustomobject]@{ Name = 'Doors'; Size = 42; Tags = @('a', 'b') }
    $o | ConvertTo-Json -Compress
}
Step 4 'strings' {
    $s = 'Macrohard Doors 2026'
    "$($s -replace '\d+', 'N')|$(($s -split ' ').Count)|$($s.ToUpperInvariant())|$('{0:D5}' -f 42)"
}
Step 5 'functions' {
    function Fib([int]$n) { if ($n -lt 2) { $n } else { (Fib ($n - 1)) + (Fib ($n - 2)) } }
    $caught = try { throw [System.InvalidOperationException]::new('boom') } catch [InvalidOperationException] { $_.Exception.Message }
    "$(Fib 15) $caught"
}
Step 6 'collections' {
    $h = @{}
    foreach ($w in 'pear apple fig apple pear apple'.Split(' ')) { $h[$w] = 1 + [int]$h[$w] }
    ($h.GetEnumerator() | Sort-Object Name | ForEach-Object { "$($_.Name)=$($_.Value)" }) -join ','
}
Step 7 'sorting' {
    $groups = 'delta', 'Alpha', 'charlie', 'Bravo', 'alpha' | Sort-Object | Group-Object { $_.ToLowerInvariant() }
    ($groups | ForEach-Object { "$($_.Name):$($_.Count)" }) -join ','
}
Step 8 'files' {
    $dir = Join-Path ([System.IO.Path]::GetTempPath()) 'pwsh-a'
    New-Item -ItemType Directory -Path $dir -Force | Out-Null
    $file = Join-Path $dir 'probe.txt'
    Set-Content -Path $file -Value 'one', 'two'
    Add-Content -Path $file -Value 'three'
    $lines = Get-Content -Path $file
    $length = (Get-Item $file).Length
    $listed = (Get-ChildItem -Path $dir -File).Name -join '|'
    Remove-Item -Path $dir -Recurse -Force
    "$($lines.Count) $length $listed $(Test-Path $file)"
}
Step 9 'dotnet' {
    $r = [math]::Round([math]::Sqrt(2), 6)
    $b64 = [Convert]::ToBase64String([Text.Encoding]::UTF8.GetBytes('Doors'))
    "$r $b64 $([Environment]::Is64BitProcess)"
}
Step 10 'commands' {
    $c = Get-Command -Name Get-ChildItem
    $d = Get-Date -Year 2026 -Month 11 -Day 16 -Hour 0 -Minute 0 -Second 0
    "$($c.CommandType) $($d.ToString('yyyy-MM-dd', [Globalization.CultureInfo]::InvariantCulture))"
}
Step 11 'scriptblocks' {
    $sb = { param($x) $x * 3 }
    $r = & $sb 14
    $f = Invoke-Expression '6 * 7'
    "$r $f"
}
if ($script:failures -eq 0) { 'PWSH-A PASSED' } else { 'PWSH-A FAILED' }
