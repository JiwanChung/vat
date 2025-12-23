# VHS Demo for vat
# https://github.com/charmbracelet/vhs

Output demo.gif

Set Shell "bash"
Set FontSize 16
Set Width 900
Set Height 500
Set Theme "Catppuccin Mocha"
Set Padding 20
Set TypingSpeed 50ms

# JSON Demo
Type "vat examples/sample.json"
Sleep 400ms
Enter
Sleep 1.2s

# Navigate down one at a time
Type "j"
Sleep 150ms
Type "j"
Sleep 150ms
Type "j"
Sleep 150ms
Type "j"
Sleep 400ms

# Collapse node
Enter
Sleep 600ms

# Expand
Enter
Sleep 600ms

# Search
Type "/"
Sleep 200ms
Type@30ms "engines"
Enter
Sleep 800ms

Type "q"
Sleep 400ms

# CSV Demo
Type "vat examples/users.csv"
Sleep 400ms
Enter
Sleep 1.2s

# Navigate
Type "j"
Sleep 150ms
Type "j"
Sleep 150ms
Type "j"
Sleep 400ms

# Search
Type "/"
Sleep 200ms
Type@30ms "Dev"
Enter
Sleep 800ms

Type "n"
Sleep 500ms

Type "q"
Sleep 400ms

# Cargo.toml Demo
Type "vat Cargo.toml"
Sleep 400ms
Enter
Sleep 1.2s

Type "j"
Sleep 100ms
Type "j"
Sleep 100ms
Type "j"
Sleep 100ms
Type "j"
Sleep 100ms
Type "j"
Sleep 400ms

Enter
Sleep 500ms

Type "q"
Sleep 500ms
