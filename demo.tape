# VHS Demo for vat
# https://github.com/charmbracelet/vhs

Output demo.gif

Set Shell "bash"
Set FontSize 16
Set Width 900
Set Height 500
Set Theme "Catppuccin Mocha"
Set Padding 20

# JSON Demo
Type "vat examples/sample.json"
Sleep 400ms
Enter
Sleep 1.2s

# Navigate
Type "jjjj"
Sleep 600ms

# Collapse node
Enter
Sleep 600ms

# Expand
Enter
Sleep 600ms

# Search
Type "/"
Sleep 200ms
Type "engines"
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
Type "jjj"
Sleep 600ms

# Search
Type "/"
Sleep 200ms
Type "Developer"
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

Type "jjjjj"
Sleep 600ms

Enter
Sleep 500ms

Type "q"
Sleep 500ms
