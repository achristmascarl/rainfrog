#!/bin/sh

main() {
  need_cmd "curl"
  need_cmd "jq"
  need_cmd "fzf"

  temp="$(mktemp -d "/tmp/rainfrog-install-XXXXXX")"
  echo "temp dir: $temp"

  echo "installing 🐸 rainfrog..."
  release_json=$(curl -s https://api.github.com/repos/achristmascarl/rainfrog/releases/latest | jq)
  binary=$(jq <<<$release_json | jq -r '.["assets"] | .[] | .name' | sed -u "s/^.*[.sha256]$//" | sed -u "s/[.]tar[.]gz//" | awk 'NF' | fzf --header "choose a binary from the latest rainfrog release:" --reverse)
  if [ -z "$binary" ]; then
    echo "no binary selected"
    exit 1
  fi
  echo "selected binary: $binary"

  # make sure local bin dir exists
  mkdir -p "$HOME/.local/bin"

  # download binary and hash
  echo "downloading binary and hash..."
  curl -fL $(jq <<<$release_json | jq -r ".assets[] | select(.name | contains(\"$binary.tar.gz\")) | .browser_download_url") > "$temp/$binary.tar.gz"
  curl -fL $(jq <<<$release_json | jq -r ".assets[] | select(.name | contains(\"$binary.sha256\")) | .browser_download_url") > "$temp/$binary.sha256"
  current=$(pwd)
  cd $temp
  shasum -a 256 -c "$temp/$binary.sha256" --strict
  sha256check=$?
  cd $current
  if [ $sha256check -ne 0 ]; then
    echo "sha256 check failed"
    exit 1
  fi

  # clean up and unpack
  rm -rf "$HOME/.local/rainfrog"
  mkdir -p "$HOME/.local/rainfrog"
  tar -xzf "$temp/$binary.tar.gz" -C "$HOME/.local/"

  # link binary
  ln -sf "$HOME/.local/rainfrog" "$HOME/.local/bin/rainfrog"

  # check installation and PATH
  echo ""
  if [ "$(which "rainfrog")" = "$HOME/.local/bin/rainfrog" ]; then
        echo "rainfrog was successfully installed! 🎊"
    else
        echo " to run rainfrog from the terminal, you must add ~/.local/bin to your PATH"
        echo "you can run rainfrog now with '~/.local/bin/rainfrog'"
    fi
}

# ty rustup for these
need_cmd() {
  if ! check_cmd "$1"; then
    echo "need '$1' (command not found)" >&2
  fi
}

check_cmd() {
  command -v "$1" >/dev/null 2>&1
}

main "$@" || exit 1
