Output demos/out/quickstart.gif

Require cargo

Set Shell bash
Set Width 1200
Set Height 720
Set FontSize 24
Set TypingSpeed 35ms
Set Theme "Catppuccin Mocha"

Type "cargo build -q -p jgrep && alias jgrep=./target/debug/jgrep"
Enter
Sleep 2s

Type "jgrep '.name' demos/fixtures/users.ndjson"
Enter
Wait+Line /Alice/
Sleep 1s

Type "jgrep 'select(.age >= 18) | .name' demos/fixtures/users.ndjson"
Enter
Wait+Line /Carla/
Sleep 2s
