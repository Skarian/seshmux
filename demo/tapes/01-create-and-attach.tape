Output "demo/output/01-create-and-attach.gif"

Require bash
Require git
Require tmux

Set Shell "zsh"
Set Theme "Catppuccin Mocha"
Set Width 1280
Set Height 840
Set FontSize 22
Set FontFamily "VictorMono Nerd Font"
Set TypingSpeed 65ms
Set PlaybackSpeed 0.60

Hide
Type "cd demo/tmp/repo-create"
Enter
Type "./demo-tmux new-session -s seshmux-demo-01 -c ."
Enter
Sleep 450ms
Ctrl+b
Sleep 400ms
Type ":clear-history"
Enter
Sleep 400ms
Ctrl+l
Show

Sleep 450ms
Type "seshmux"
Enter
Sleep 1000ms

Enter
Sleep 850ms

Type "demo-feature"
Sleep 350ms
Enter
Sleep 850ms

Enter
Sleep 850ms

Enter
Sleep 850ms

Enter
Sleep 850ms

Enter
Sleep 1550ms
