Output demos/out/pretty-and-custom-color.gif

Require cargo

Set Shell bash
Set Width 1200
Set Height 760
Set FontSize 22
Set TypingSpeed 35ms
Set Theme "Catppuccin Mocha"

Type "cargo build -q -p jgrep"
Enter
Sleep 2s

Type "./target/debug/jgrep --pretty '{name, role, city: .address.city}' demos/fixtures/users.ndjson"
Enter
Wait+Screen /\"city\": \"Berlin\"/
Wait+Screen /\"role\": \"operator\"/
Sleep 1s

Type "./target/debug/jgrep -C --color-level-field app.level '{level:.app.level,message,trace_id}' demos/fixtures/app-logs.ndjson"
Enter
Wait+Screen /cache unavailable/
Wait+Screen /cache refresh slow/
Sleep 2s
