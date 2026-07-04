Output demos/out/explorer-snapshot.gif

Require cargo
Require head

Set Shell bash
Set Width 1200
Set Height 760
Set FontSize 22
Set TypingSpeed 35ms
Set Theme "Catppuccin Mocha"

Type "cargo build -q -p jgrep"
Enter
Sleep 2s

Type "./target/debug/jgrep explore --print --filter status=active demos/fixtures/users.ndjson"
Enter
Wait+Screen /generated jq/
Sleep 1s

Type "./target/debug/jgrep completion bash | head -12"
Enter
Wait+Screen /COMPREPLY/
Sleep 2s
