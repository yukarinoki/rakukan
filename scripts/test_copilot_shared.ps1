param([int]$Action = 6, [string]$Text = '', [string]$Instructions = '', [int]$TimeoutSeconds = 120)
$ErrorActionPreference = 'Stop'
$pipe = [IO.Pipes.NamedPipeClientStream]::new('.', 'AITextService', [IO.Pipes.PipeDirection]::InOut, [IO.Pipes.PipeOptions]::Asynchronous)
$cancel = [Threading.CancellationTokenSource]::new($TimeoutSeconds * 1000)
$watch = [Diagnostics.Stopwatch]::StartNew()
try {
    $pipe.Connect(3000)
    $id = [Guid]::NewGuid().ToString()
    $body = [Text.Encoding]::UTF8.GetBytes((@{ requestId=$id; action=$Action; text=$Text; instructions=$Instructions; sourceLang='ja'; targetLang='en'; lang='ja' } | ConvertTo-Json -Compress))
    $header = [BitConverter]::GetBytes([int]$body.Length)
    $pipe.Write($header,0,4)
    $pipe.Write($body,0,$body.Length)
    $pipe.Flush()
    function Read-Exact([byte[]]$buffer) {
        $offset=0
        while ($offset -lt $buffer.Length) {
            $n=$pipe.ReadAsync($buffer,$offset,$buffer.Length-$offset,$cancel.Token).GetAwaiter().GetResult()
            if ($n -eq 0) { throw 'Unexpected EOF' }
            $offset += $n
        }
    }
    Read-Exact $header
    $size=[BitConverter]::ToInt32($header,0)
    if ($size -lt 1 -or $size -gt 1048576) { throw 'Invalid response size' }
    $response=[byte[]]::new($size)
    Read-Exact $response
    $result=[Text.Encoding]::UTF8.GetString($response) | ConvertFrom-Json
    if ($result.requestId -ne $id) { throw 'Request ID mismatch' }
    if (!$result.success) { throw $result.error }
    [pscustomobject]@{ elapsed_ms=$watch.ElapsedMilliseconds; response=$result } | ConvertTo-Json -Depth 8
} finally { $pipe.Dispose(); $cancel.Dispose() }
