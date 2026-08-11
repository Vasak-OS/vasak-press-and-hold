# Maintainer: Vasak Group
pkgname=vasak-press-and-hold
pkgver=0.1.0
pkgrel=1
pkgdesc="Press & Hold Accents daemon - hold a key to get accented character variants on Wayland"
arch=('x86_64' 'aarch64')
url="https://github.com/VasakOS/vasak-press-and-hold"
license=('MIT')
depends=(
  'gtk3'
  'webkit2gtk-4.1'
  'libxkbcommon'
  'libappindicator-gtk3'
)
makedepends=(
  'cargo'
  'bun'
  'webkit2gtk-4.1'
  'libappindicator-gtk3'
  'libxkbcommon'
  'gtk3'
)
install=vasak-press-and-hold.install
source=("$pkgname-$pkgver.tar.gz::https://github.com/VasakOS/$pkgname/archive/refs/tags/v$pkgver.tar.gz"
        "99-vasak-press-and-hold.rules")
sha256sums=('SKIP' 'SKIP')

prepare() {
  cd "$pkgname-$pkgver"
  export RUSTUP_TOOLCHAIN=stable
  cargo fetch --locked --target "$(rustc -vV | sed -n 's/host: //p')"
}

build() {
  cd "$pkgname-$pkgver"
  export RUSTUP_TOOLCHAIN=stable
  export RUSTFLAGS="-C link-arg=-fuse-ld=lld"
  bun install --frozen-lockfile
  bun run tauri build
}

package() {
  cd "$pkgname-$pkgver"

  # Install binary
  install -Dm755 "src-tauri/target/release/vasak-press-and-hold" \
    "$pkgdir/usr/bin/vasak-press-and-hold"

  # Install udev rules for input device access
  install -Dm644 "$srcdir/99-vasak-press-and-hold.rules" \
    "$pkgdir/usr/lib/udev/rules.d/99-vasak-press-and-hold.rules"

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
