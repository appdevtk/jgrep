Output demos/out/explorer-snapshot.gif
Output demos/out/explorer-snapshot.mp4

Require cargo
Require head

Set Shell bash
Set Width 1200
Set Height 760
Set FontSize 22
Set TypingSpeed 35ms
Set Theme "Catppuccin Mocha"

Hide
Type "cargo build -q --locked -p jgrep"
Enter
Sleep 2s
Show

Type "./target/debug/jgrep explore --print --filter status=active demos/fixtures/users.ndjson"
Enter
Wait+Screen /generated jq/
Sleep 1s

Type "./target/debug/jgrep completion bash | head -12"
Enter
Wait+Screen /COMPREPLY/
Sleep 2s
