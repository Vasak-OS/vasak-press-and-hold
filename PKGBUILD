# Maintainer: Vasak Group
pkgname=vasak-press-and-hold
pkgver=0.1.1
pkgrel=2
pkgdesc="Press & Hold Accents daemon - hold a key to get accented character variants on Wayland"
arch=('x86_64' 'aarch64')
url="https://github.com/Vasak-OS/$pkgname"
license=('MIT')
depends=(
  'gtk3'
  'webkit2gtk-4.1'
  'libxkbcommon'
  'libappindicator-gtk3'
  # The picker is drawn as a layer-shell surface so it can place itself.
  'gtk-layer-shell'
)
makedepends=(
  'git'
  'cargo'
  'bun'
  'webkit2gtk-4.1'
  'libappindicator-gtk3'
  'libxkbcommon'
  'gtk3'
)
install=vasak-press-and-hold.install
# git+ like every other VasakOS package: the tarball form needed a release tag
# that was never cut, so the package could not be built at all.
source=("git+${url}.git")
sha256sums=('SKIP')

build() {
  cd "$srcdir/$pkgname"
  export RUSTUP_TOOLCHAIN=stable
  export RUSTFLAGS="-C link-arg=-fuse-ld=lld"
  bun install --frozen-lockfile
  bun run tauri build
}

package() {
  cd "$srcdir/$pkgname"

  # Install binary
  install -Dm755 "src-tauri/target/release/vasak-press-and-hold" \
    "$pkgdir/usr/bin/vasak-press-and-hold"

  # Install udev rules for input device access
  install -Dm644 "60-vasak-press-and-hold.rules" \
    "$pkgdir/usr/lib/udev/rules.d/60-vasak-press-and-hold.rules"

  # Loading uinput is what turns /dev/uinput into a real device; until then it
  # is a static node the rules above cannot reach.
  install -Dm644 "packaging/$pkgname.modules-load.conf" \
    "$pkgdir/usr/lib/modules-load.d/$pkgname.conf"

  # Nothing started the daemon before this: no unit, no autostart entry. The
  # feature was installed and simply never ran, which looks exactly like it not
  # working.
  install -Dm644 "packaging/$pkgname.service" \
    "$pkgdir/usr/lib/systemd/user/$pkgname.service"

  # Install icons if present
  if [ -d src-tauri/icons ]; then
    for size in 32x32 128x128 128x128@2x; do
      if [ -f "src-tauri/icons/${size}.png" ]; then
        install -Dm644 "src-tauri/icons/${size}.png" \
          "$pkgdir/usr/share/icons/hicolor/${size}/apps/vasak-press-and-hold.png"
      fi
    done
  fi
}
