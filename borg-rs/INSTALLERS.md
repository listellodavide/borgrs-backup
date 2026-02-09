## Borg-gui run
   cargo run -p borg-gui

## Installers

### Arch Linux (.pkg.tar.zst)
   cd borg-gui/pkgbuild
   Then run makepkg -si to build and install.
### Debian Linux (.deb)
   cargo install cargo-deb
   cargo deb -p borg-gui

### Fedora/RHEL (.rpm)
   cargo install cargo-generate-rpm
   cargo build --release -p borg-gui
   cargo generate-rpm -p borg-gui

### MacOS (.dmg/.app)
   cargo install cargo-bundle
   cargo bundle --release -p borg-gui --target aarch64-apple-darwin  # For Apple Silicon
   cargo bundle --release -p borg-gui --target x86_64-apple-darwin   # For Intel Mac

### Windows (.msi) 
   cargo install cargo-wix
   cargo wix -p borg-gui
    