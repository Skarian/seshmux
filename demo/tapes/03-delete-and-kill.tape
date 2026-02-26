Output "demo/output/03-delete-and-kill.gif"

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
Type "cd demo/tmp/repo-delete"
Enter
Type "./demo-tmux new-session -s seshmux-demo-03 -c ."
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

Down
Sleep 213ms
Down
Sleep 213ms
Down
Sleep 213ms
Enter
Sleep 550ms

Enter
Sleep 900ms

Space
Sleep 400ms
Down
Sleep 400ms
Space
Sleep 400ms
Enter
Sleep 900ms

Space
Sleep 400ms
Enter
Sleep 1700ms
