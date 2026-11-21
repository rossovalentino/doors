# Manual output cleanup

The exact previously reviewed deletion list is `out/cleanup-plan.json`:54,365
files,64,775,849,126 logical bytes (about60.3GiB). Existing passed/historical
reports and latest launcher references were followed before producing that list.
No files were deleted by the agent. Earlier NTFS compression reduced physical
storage; the logical-byte total is not a promise of equal additional free space.
New P11 build/test outputs are outside that earlier plan.

Run this in PowerShell to delete only those listed files. It excludes the owner
disk and acquired firmware/tools even if a list entry were changed accidentally.

```powershell
$root = 'C:\Users\win\Desktop\Doors OS\door-os'
$out = [IO.Path]::GetFullPath("$root\out")
$plan = Get-Content -LiteralPath "$out\cleanup-plan.json" -Raw | ConvertFrom-Json
foreach ($item in $plan.files) {
    $path = [IO.Path]::GetFullPath((Join-Path $root $item.path))
    if (!$path.StartsWith("$out\", [StringComparison]::OrdinalIgnoreCase) -or
        $path.StartsWith("$out\session\", [StringComparison]::OrdinalIgnoreCase) -or
        $path.StartsWith("$out\acquisitions\", [StringComparison]::OrdinalIgnoreCase)) {
        throw "Protected or invalid cleanup path: $path"
    }
    if (Test-Path -LiteralPath $path -PathType Leaf) {
        Remove-Item -LiteralPath $path -Force
    }
}
```

Keep `out/session`, `out/acquisitions`, `reports`, the latest image and its
referenced inputs. Future cache cleanup needs a fresh reference-closure plan;
deleting all of `out` would remove user data, firmware and retained binary evidence.
