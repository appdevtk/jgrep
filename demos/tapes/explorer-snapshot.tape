Output demos/out/explorer-snapshot.gif

Require cargo
Require sed

Set Shell bash
Set Width 1200
Set Height 760
Set FontSize 22
Set TypingSpeed 35ms
Set Theme "Catppuccin Mocha"

Type "cargo build -q -p jgrep && alias jgrep=./target/debug/jgrep"
Enter
Sleep 2s

Type "jgrep explore --print --filter status=active demos/fixtures/users.ndjson"
Enter
Wait+Line /generated jq/
Sleep 1s

Type "jgrep completion bash | sed -n '1,12p'"
Enter
Wait+Line /COMPREPLY/
Sleep 2s
