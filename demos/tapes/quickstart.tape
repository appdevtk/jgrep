Output demos/out/quickstart.gif

Require cargo

Set Shell bash
Set Width 1200
Set Height 720
Set FontSize 24
Set TypingSpeed 35ms
Set Theme "Catppuccin Mocha"

Type "cargo build -q -p jgrep"
Enter
Sleep 2s

Type "./target/debug/jgrep -p name demos/fixtures/users.ndjson"
Enter
Wait+Screen /Alice/
Sleep 1s

Type "./target/debug/jgrep -f demos/fixtures/adults.jq demos/fixtures/users.ndjson"
Enter
Wait+Screen /Carla/
Sleep 2s
