#!/bin/sh
# Install grounds from its GitHub releases.
#
#   curl -fsSL https://raw.githubusercontent.com/mihaidaniel34/grounds/master/install.sh | sh
#
# Options (as arguments after `sh -s --`, or as environment variables):
#   --version X.Y.Z   GROUNDS_VERSION       a release; default: the latest
#   --dir DIR         GROUNDS_INSTALL_DIR   where the binary goes; default: ~/.local/bin
#
# It picks the build for this machine (macOS or Linux, arm64 or x86_64),
# checks it against the release's SHA-256 file, and copies `grounds` into DIR.
# It never asks for sudo: pick a DIR you can write to.
#
# The whole script is one function, called on the last line, so a download cut
# short runs nothing.

main() {
	set -eu
	repo=mihaidaniel34/grounds
	version=${GROUNDS_VERSION:-}
	dir=${GROUNDS_INSTALL_DIR:-}

	while [ $# -gt 0 ]; do
		case $1 in
		--version)
			version=${2:?--version needs a value}
			shift 2
			;;
		--version=*)
			version=${1#*=}
			shift
			;;
		--dir)
			dir=${2:?--dir needs a value}
			shift 2
			;;
		--dir=*)
			dir=${1#*=}
			shift
			;;
		-h | --help)
			usage
			return 0
			;;
		*) fail "unknown option: $1 (try --help)" ;;
		esac
	done
	[ -n "$dir" ] || dir=${HOME:?HOME is not set; choose a directory with --dir}/.local/bin

	if command -v curl >/dev/null 2>&1; then
		fetch() { curl -fsSL --proto '=https' --tlsv1.2 --retry 3 -o "$2" "$1"; }
		# github.com/<repo>/releases/latest redirects to .../tag/v<version>;
		# following it needs no API token and has no rate limit.
		latest_tag() {
			u=$(curl -fsSLI --proto '=https' --tlsv1.2 -o /dev/null -w '%{url_effective}' \
				"https://github.com/$repo/releases/latest") && echo "${u##*/tag/}"
		}
	elif command -v wget >/dev/null 2>&1; then
		# GNU wget checks TLS certificates. Busybox's does only through Alpine's
		# ssl_client helper; its built-in TLS and its openssl fallback do not,
		# and the checksum comes over the same connection, so refuse those.
		case $(wget --version 2>/dev/null) in
		*"GNU Wget"*) ;;
		*) command -v ssl_client >/dev/null 2>&1 ||
			fail "this wget may not check TLS certificates; install curl (or GNU wget) and run again" ;;
		esac
		fetch() { wget -q -O "$2" "$1"; }
		# The release redirect needs headers wget prints differently per
		# implementation; the API is limited to 60 requests an hour per address.
		latest_tag() {
			wget -q -O - "https://api.github.com/repos/$repo/releases/latest" |
				sed -n 's/.*"tag_name": *"\([^"]*\)".*/\1/p' | head -n 1
		}
	else
		fail "neither curl nor wget is installed"
	fi

	target=$(detect_target)

	if [ -z "$version" ]; then
		version=$(latest_tag) || version=
		[ -n "$version" ] || fail "could not find the latest release; name one with --version X.Y.Z"
		case ${version#v} in
		"" | */* | *[!0-9A-Za-z.+-]*) fail "could not read the latest release's version (got '$version')" ;;
		esac
	fi
	version=${version#v}

	name=grounds-$version-$target
	url=https://github.com/$repo/releases/download/v$version/$name.tar.gz

	tmp=$(mktemp -d 2>/dev/null || mktemp -d -t grounds)
	stage=
	trap 'rm -rf "$tmp" ${stage:+"$stage"}' EXIT
	trap 'exit 130' INT
	trap 'exit 143' TERM

	say "downloading grounds $version for $target"
	fetch "$url" "$tmp/$name.tar.gz" || fail "could not download $url (is there a release $version with a build for $target?)"
	fetch "$url.sha256" "$tmp/$name.tar.gz.sha256" || fail "could not download $url.sha256"

	want=$(cut -d ' ' -f 1 <"$tmp/$name.tar.gz.sha256")
	got=$(sha256 "$tmp/$name.tar.gz")
	[ -n "$want" ] && [ "$want" = "$got" ] ||
		fail "checksum mismatch for $name.tar.gz: expected $want, got $got"

	tar -xzf "$tmp/$name.tar.gz" -C "$tmp"
	[ -f "$tmp/$name/grounds" ] || fail "the archive has no $name/grounds"

	mkdir -p "$dir" 2>/dev/null || fail "cannot create $dir; choose another with --dir"
	[ -w "$dir" ] || fail "cannot write to $dir; choose another with --dir (or run with sudo)"
	# Copy to a new file of our own next to the target and rename it over the
	# target, so a running grounds (the daemon) keeps its file, nobody sees a
	# half-written one, and nothing someone else put in DIR is written through.
	stage=$(mktemp "$dir/.grounds.XXXXXX") || fail "cannot create a file in $dir"
	cp "$tmp/$name/grounds" "$stage"
	chmod 755 "$stage"
	mv -f "$stage" "$dir/grounds"
	stage=

	installed=$("$dir/grounds" --version 2>/dev/null) ||
		fail "$dir/grounds was written but does not run (is $dir on a noexec mount?); choose another directory with --dir"
	say "installed $installed to $dir/grounds"

	case :${PATH:-}: in
	*:"$dir":* | *:"${dir%/}":*) ;;
	*)
		say "$dir is not on your PATH; add it, for example:"
		case $(basename "${SHELL:-sh}") in
		fish) say "  fish_add_path $dir" ;;
		zsh) say "  echo 'export PATH=\"$dir:\$PATH\"' >> ~/.zshrc" ;;
		*) say "  echo 'export PATH=\"$dir:\$PATH\"' >> ~/.profile" ;;
		esac
		;;
	esac

	if [ -z "${JAVA_HOME:-}" ] && ! command -v java >/dev/null 2>&1 &&
		! { [ "$(uname -s)" = Darwin ] && /usr/libexec/java_home >/dev/null 2>&1; }; then
		say "note: no JDK found. grounds does not run Java, but it reads the JDK's API;"
		say "      without one most rules have little to work with. Install a JDK 17+ or set JAVA_HOME."
	fi
}

detect_target() {
	os=$(uname -s)
	arch=$(uname -m)
	case $arch in
	arm64 | aarch64) arch=aarch64 ;;
	x86_64 | amd64) arch=x86_64 ;;
	*) fail "no prebuilt grounds for $arch; build it with: cargo install --locked --git https://github.com/$repo grounds-cli" ;;
	esac
	case $os in
	Darwin)
		# A shell running under Rosetta reports x86_64 on Apple silicon; the
		# native build is the one to install there.
		if [ "$arch" = x86_64 ] && [ "$(sysctl -n sysctl.proc_translated 2>/dev/null)" = 1 ]; then
			arch=aarch64
		fi
		echo "$arch-apple-darwin"
		;;
	Linux) echo "$arch-unknown-linux-musl" ;;
	*) fail "no prebuilt grounds for $os; build it with: cargo install --locked --git https://github.com/$repo grounds-cli" ;;
	esac
}

sha256() {
	if command -v sha256sum >/dev/null 2>&1; then
		sha256sum "$1" | cut -d ' ' -f 1
	elif command -v shasum >/dev/null 2>&1; then
		shasum -a 256 "$1" | cut -d ' ' -f 1
	elif command -v openssl >/dev/null 2>&1; then
		openssl dgst -sha256 -r "$1" | cut -d ' ' -f 1
	else
		fail "no sha256sum, shasum or openssl to check the download with"
	fi
}

usage() {
	cat <<'EOF'
Install grounds from its GitHub releases.

  curl -fsSL https://raw.githubusercontent.com/mihaidaniel34/grounds/master/install.sh | sh -s -- [options]

  --version X.Y.Z   a release (or GROUNDS_VERSION); default: the latest
  --dir DIR         where the binary goes (or GROUNDS_INSTALL_DIR); default: ~/.local/bin
EOF
}

say() { printf 'grounds-install: %s\n' "$*" >&2; }
fail() {
	say "error: $*"
	exit 1
}

main "$@"
