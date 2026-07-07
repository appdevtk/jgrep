Output demos/out/explore-tui-streaming.gif

Require cargo
Require bash

Set Shell bash
Set Width 1200
Set Height 760
Set FontSize 22
Set TypingSpeed 35ms
Set Theme "Catppuccin Mocha"

Type "cargo build -q -p jgrep"
Enter
Sleep 2s

Type "JGREP_DEMO_LOOPS=5 JGREP_DEMO_DELAY=0.24 demos/scripts/k8s-log-stream.sh | ./target/debug/jgrep explore"
Enter
Wait+Screen /stream/
Sleep 1s

Type "log.level=ERROR"
Wait+Screen /declined/
Wait+Screen /fallback/
Wait+Screen /stream/
Ctrl+F
Ctrl+L
Ctrl+G
Ctrl+O
Sleep 500ms
Type "{level:.log.level,message,trace_id}"
Wait+Screen /payment declined/
Wait+Screen /fallback charge failed/
Wait+Screen /trace_id/
Wait+Screen /logs:full/
Sleep 3s

Escape
Sleep 1s
