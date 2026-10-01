Output demos/out/explore-tui-streaming.gif
Output demos/out/explore-tui-streaming.mp4

Require cargo
Require bash

Set Shell bash
Set Width 1600
Set Height 1000
Set FontSize 22
Set TypingSpeed 35ms
Set Theme "Catppuccin Mocha"

Hide
Type "cargo build -q --locked -p jgrep"
Enter
Sleep 2s
Show

Type "JGREP_DEMO_LOOPS=8 JGREP_DEMO_DELAY=0.30 demos/scripts/k8s-log-stream.sh | ./target/debug/jgrep explore"
Enter
Wait+Screen /stream/
Sleep 1s

Type "log.level=ERROR"
Wait+Screen /declined/
Wait+Screen /fallback/
Wait+Screen /stream/
Ctrl+F
Ctrl+L
Wait+Screen /color:on/
Ctrl+K
Wait+Screen /color:off/
Sleep 1s
Ctrl+K
Wait+Screen /color:on/
Ctrl+G
Ctrl+O
Sleep 500ms
Type "{level:.log.level,message,trace_id:.trace.id}"
Wait+Screen /payment declined/
Wait+Screen /fallback charge failed/
Wait+Screen /trace_id/
Wait+Screen /trc-/
Wait+Screen /logs:full/
Wait+Screen /stream complete/
Down
Down
Down
Wait+Screen /scroll:4/
Sleep 6s

Escape
Sleep 1s
