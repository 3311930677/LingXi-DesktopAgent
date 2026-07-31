param(
    [int]$Port = 9527
)

$ErrorActionPreference = "Stop"

function Invoke-ImeQuery {
    param([string]$Pinyin)

    $client = [System.Net.Sockets.TcpClient]::new()
    try {
        $client.Connect("127.0.0.1", $Port)
        $stream = $client.GetStream()
        $encoding = [System.Text.UTF8Encoding]::new($false)
        $writer = [System.IO.StreamWriter]::new($stream, $encoding)
        $reader = [System.IO.StreamReader]::new($stream, $encoding)
        $writer.AutoFlush = $true
        $request = @{ type = "query"; pinyin = $Pinyin; limit = 8 } | ConvertTo-Json -Compress
        $writer.WriteLine($request)
        $line = $reader.ReadLine()
        return $line | ConvertFrom-Json
    }
    finally {
        $client.Dispose()
    }
}

# Keep this Windows PowerShell 5 compatible: ASCII source avoids UTF-8 BOM
# ambiguity; expected Chinese strings are constructed from Unicode code points.
$Cases = @(
    @{ Pinyin = "nihao"; Expected = -join ([char]0x4F60, [char]0x597D) },
    @{ Pinyin = "suoyi"; Expected = -join ([char]0x6240, [char]0x4EE5) },
    @{ Pinyin = "dang"; Expected = [string][char]0x5F53 },
    @{ Pinyin = "zhong"; Expected = [string][char]0x4E2D },
    @{ Pinyin = "ren"; Expected = [string][char]0x4EBA },
    @{ Pinyin = "de"; Expected = [string][char]0x7684 }
)

$Failed = 0
foreach ($case in $Cases) {
    try {
        $response = Invoke-ImeQuery -Pinyin $case.Pinyin
        $texts = @($response.candidates | ForEach-Object { $_.text })
        $first = if ($texts.Count -gt 0) { $texts[0] } else { "EMPTY" }
        if ($first -eq $case.Expected) {
            Write-Host ("PASS  {0,-8} -> {1}" -f $case.Pinyin, ($texts -join " / ")) -ForegroundColor Green
        } else {
            Write-Host ("FAIL  {0,-8} expected {1}, got {2}" -f $case.Pinyin, $case.Expected, ($texts -join " / ")) -ForegroundColor Red
            $Failed++
        }
    }
    catch {
        Write-Host "Cannot connect to ime-server on 127.0.0.1:$Port" -ForegroundColor Red
        Write-Host "Start the project with .\scripts\run-dev.ps1 first."
        exit 1
    }
}

if ($Failed -gt 0) {
    Write-Host "$Failed candidate checks failed." -ForegroundColor Red
    exit 1
}

Write-Host "All IME candidate checks passed." -ForegroundColor Green
