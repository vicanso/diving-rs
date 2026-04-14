#!/usr/bin/env sh

set -eu
printf '\n'

BOLD="$(tput bold 2>/dev/null || printf '')"
GREY="$(tput setaf 0 2>/dev/null || printf '')"
GREEN="$(tput setaf 2 2>/dev/null || printf '')"
YELLOW="$(tput setaf 3 2>/dev/null || printf '')"
BLUE="$(tput setaf 4 2>/dev/null || printf '')"
RED="$(tput setaf 1 2>/dev/null || printf '')"
NO_COLOR="$(tput sgr0 2>/dev/null || printf '')"

info() {
  printf '%s\n' "${BOLD}${GREY}>${NO_COLOR} $*"
}

error() {
  printf '%s\n' "${RED}x $*${NO_COLOR}" >&2
}

completed() {
  printf '%s\n' "${GREEN}✓${NO_COLOR} $*"
}

has() {
  command -v "$1" 1>/dev/null 2>&1
}

get_latest_release() {
  curl --silent "https://api.github.com/repos/vicanso/diving-rs/releases/latest" |
    grep '"tag_name":' |
    sed -E 's/.*"([^"]+)".*/\1/'
}

detect_platform() {
  platform="$(uname -s)"
  case "${platform}" in
    Linux*) platform="linux" ;;
    Darwin*) platform="darwin" ;;
    MINGW*|MSYS*|CYGWIN*) platform="windows" ;;
    *)
      error "Unsupported platform: ${platform}"
      exit 1
      ;;
  esac
  printf '%s' "${platform}"
}

detect_arch() {
  arch="$(uname -m)"
  case "${arch}" in
    x86_64) arch="x86_64" ;;
    aarch64|arm64) arch="aarch64" ;;
    *)
      error "Unsupported architecture: ${arch}"
      exit 1
      ;;
  esac
  printf '%s' "${arch}"
}

download_and_install() {
  version="$1"
  platform="$2"
  arch="$3"

  # 直接构建二进制文件名，diving-rs 遵循 diving-{platform}-{arch}
  if [ "${platform}" = "windows" ]; then
    filename="diving-windows.exe"
  else
    # Darwin 或其他
    # 如果 Darwin 也是 aarch64/x86_64 结构
    filename="diving-${platform}-${arch}"
  fi

  url="https://github.com/vicanso/diving-rs/releases/download/${version}/${filename}"
  target_bin="/usr/local/bin/diving"

  info "Downloading diving-rs ${version}..."
  info "URL: ${url}"

  # 下载到临时文件
  tmp_file="./diving_install_tmp"
  
  if has curl; then
    curl -sSL "${url}" -o "${tmp_file}"
  elif has wget; then
    wget -q "${url}" -O "${tmp_file}"
  else
    error "curl or wget not found."
    exit 1
  fi

  if [ ! -s "${tmp_file}" ]; then
    error "Downloaded file is empty. Please check if the version/platform is correct."
    rm -f "${tmp_file}"
    exit 1
  fi

  chmod +x "${tmp_file}"

  info "Installing to ${target_bin}..."
  
  if [ "${platform}" = "windows" ]; then
     info "Windows detected. Binary saved as ${filename}. Please move it to your PATH."
     mv "${tmp_file}" "${filename}"
  else
    if has sudo; then
      sudo mv "${tmp_file}" "${target_bin}"
    else
      mv "${tmp_file}" "${target_bin}"
    fi
    completed "Installed successfully!"
  fi
}

main() {
  platform="$(detect_platform)"
  arch="$(detect_arch)"
  version="$(get_latest_release)"

  info "Detected: ${platform}_${arch}"
  download_and_install "${version}" "${platform}" "${arch}"
}

main