Output demos/out/yaml-and-recursive.gif

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

Type "jgrep '.spec.template.spec.containers[].image' demos/fixtures/k8s/deployment.yaml"
Enter
Wait+Line /checkout:1.7.4/
Sleep 1s

Type "jgrep -r 'select(.kind == \"Deployment\" and .spec.replicas > 2) | .metadata.name' demos/fixtures/k8s"
Enter
Wait+Line /checkout/
Sleep 2s
