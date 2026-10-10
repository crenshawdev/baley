#!/bin/sh
# Install Baley from unsigned development artifacts only.
# Issue #14 adds signature verification inside verify_download before any release.
# Running baley install at the end belongs to Build 3 T15.
set -eu

LC_ALL=C
export LC_ALL

fail() {
    printf 'baley: %s\n' "$*" >&2
    exit 1
}

valid_utf8() (
    remaining=$1
    ascii=$(printf '[\001-\177]')
    continuation=$(printf '[\200-\277]')
    two=$(printf '[\302-\337]')
    three=$(printf '[\341-\354\356-\357]')
    four=$(printf '[\361-\363]')
    while [ -n "$remaining" ]; do
        case $remaining in
            $ascii*) remaining=${remaining#?} ;;
            $two$continuation*) remaining=${remaining#??} ;;
            $(printf '\340[\240-\277]')$continuation* | \
            $three$continuation$continuation* | \
            $(printf '\355[\200-\237]')$continuation*) remaining=${remaining#???} ;;
            $(printf '\360[\220-\277]')$continuation$continuation* | \
            $four$continuation$continuation$continuation* | \
            $(printf '\364[\200-\217]')$continuation$continuation*) remaining=${remaining#????} ;;
            *) return 1 ;;
        esac
    done
)

valid_version() (
    case $1 in '' | *[!0-9.]* | .* | *. | *..*) return 1 ;; esac
    set -f
    IFS=.
    set -- $1
    [ "$#" -eq 3 ] || return 1
    for part do
        case $part in 0?*) return 1 ;; esac
        [ "${#part}" -le 20 ] || return 1
        if [ "${#part}" -eq 20 ]; then
            # Compare halves without overflowing the shell's signed integer.
            high=${part%??????????}
            low=${part#??????????}
            [ "$high" -le 1844674407 ] || return 1
            if [ "$high" -eq 1844674407 ]; then
                [ "$low" -le 3709551615 ] || return 1
            fi
        fi
    done
)

digest() {
    if command -v sha256sum >/dev/null 2>&1; then
        sum=$(sha256sum) || fail 'cannot compute SHA-256'
    else
        sum=$(shasum -a 256) || fail 'cannot compute SHA-256 with shasum -a 256'
    fi
    printf '%s\n' "${sum%% *}"
}

verify_download() {
    actual=$(digest < "$scratch/baley")
    [ "$actual" = "$sha256" ] ||
        fail "download SHA-256 is $actual, but the manifest names $sha256"
}

check_stable_path() {
    if [ -L "$stable" ]; then
        target=$(readlink "$stable") || fail "$stable: cannot read symbolic link"
        rest=${target#"$versions/"}
        folder=${rest%%/*}
        file=${rest#*/}
        if [ "$rest" = "$target" ] || [ "$file" != baley ] || ! valid_version "$folder"; then
            fail "$stable: symbolic link outside $versions/ ($target)"
        fi
        [ ! -d "$stable" ] || fail "$stable: symbolic link to a folder ($target)"
    elif [ -d "$stable" ]; then
        fail "$stable: a folder is there"
    elif [ -f "$stable" ]; then
        fail "$stable: a regular file is there"
    elif [ -e "$stable" ]; then
        fail "$stable: an entry other than a symbolic link is there"
    fi
}

[ "$#" -eq 1 ] || fail 'usage: sh install.sh https://<source>'
case $1 in https://*) source=$1 ;; *) fail 'the source must start with https://' ;; esac
while [ "${source%/}" != "$source" ]; do source=${source%/}; done

case ${HOME-} in
    '') fail 'HOME is unset or empty; set it to your absolute home folder' ;;
    /*) ;;
    *) fail 'HOME is relative; set it to your absolute home folder' ;;
esac
valid_utf8 "$HOME" || fail 'HOME is not valid UTF-8; set it to a path that is'

# Match the updater's spelling without following folder links.
install_home=
remaining=$HOME/
while [ -n "$remaining" ]; do
    component=${remaining%%/*}
    remaining=${remaining#*/}
    case $component in
        '' | .) ;;
        ..) fail 'HOME contains ..; set it to an absolute path without ..' ;;
        *) install_home=$install_home/$component ;;
    esac
done
versions=$install_home/.local/lib/crenshawdev/baley/versions
stable_folder=$install_home/.local/bin
stable=$stable_folder/baley
check_stable_path

case $(uname -s) in
    Linux) os=linux ;;
    Darwin) os=macos ;;
    *) fail 'unsupported operating system; Linux or Darwin is required' ;;
esac
case $(uname -m) in
    x86_64) arch=x86_64 ;;
    aarch64 | arm64) arch=aarch64 ;;
    *) fail 'unsupported architecture; x86_64 or aarch64 is required' ;;
esac

scratch=$(mktemp -d)
staging_temporary=
link_temporary=
cleanup() {
    [ -z "$staging_temporary" ] || rm -f "$staging_temporary"
    [ -z "$link_temporary" ] || rm -f "$link_temporary"
    rm -rf "$scratch"
}
trap cleanup 0
trap 'exit 1' HUP INT TERM

curl -q --proto '=https' --tlsv1.2 -fsSL "$source/$os-$arch/manifest" -o "$scratch/manifest" ||
    fail "manifest download failed: $source/$os-$arch/manifest"
curl -q --proto '=https' --tlsv1.2 -fsSL "$source/$os-$arch/baley" -o "$scratch/baley" ||
    fail "binary download failed: $source/$os-$arch/baley"

{
    IFS= read -r version_line || fail 'manifest line 1 is missing or has no final newline'
    IFS= read -r digest_line || fail 'manifest line 2 is missing or has no final newline'
    extra=
    if IFS= read -r extra || [ -n "$extra" ]; then
        fail 'manifest has content after line 2'
    fi
} < "$scratch/manifest"
case $version_line in
    'version '*) version=${version_line#version } ;;
    *) fail 'manifest line 1 must start with "version "' ;;
esac
valid_version "$version" || fail 'manifest line 1 needs major.minor.patch with no leading zeros and each part fitting u64'
case $digest_line in
    'sha256 '*) sha256=${digest_line#sha256 } ;;
    *) fail 'manifest line 2 must start with "sha256 "' ;;
esac
case $sha256 in *[!0-9a-f]*) fail 'manifest line 2 needs 64 lowercase hex digits' ;; esac
[ "${#sha256}" -eq 64 ] || fail 'manifest line 2 needs 64 lowercase hex digits'
# Some shells discard NUL bytes during read, so compare the exact text too.
manifest_digest=$(digest < "$scratch/manifest")
text_digest=$(printf 'version %s\nsha256 %s\n' "$version" "$sha256" | digest)
[ "$manifest_digest" = "$text_digest" ] || fail 'manifest is not exactly two UTF-8 lines'

verify_download

umask 022
staged=$versions/$version/baley
if [ -L "$staged" ]; then
    fail "$staged: staging conflict, a symbolic link is there"
elif [ -f "$staged" ]; then
    actual=$(digest < "$staged")
    [ "$actual" = "$sha256" ] || fail "$staged: staging conflict, SHA-256 is $actual, expected $sha256"
    [ -x "$staged" ] || fail "$staged: staging conflict, the file is not executable"
elif [ -e "$staged" ]; then
    fail "$staged: staging conflict, an entry other than a regular file is there"
else
    mkdir -p "$versions/$version"
    staging_temporary=$(mktemp "$versions/$version/.baley.XXXXXXXXXX")
    chmod 755 "$scratch/baley"
    mv "$scratch/baley" "$staging_temporary"
    mv -f "$staging_temporary" "$staged"
    staging_temporary=
fi

mkdir -p "$stable_folder"
temporary=$stable_folder/.${scratch##*/}
ln -s "$staged" "$temporary"
[ -L "$temporary" ] && [ "$(readlink "$temporary")" = "$staged" ] ||
    fail "$temporary: temporary symbolic link does not point to $staged"
link_temporary=$temporary
check_stable_path
mv -f "$link_temporary" "$stable"
link_temporary=

printf 'baley: placed version %s at %s\n' "$version" "$stable"
printf 'baley: versions present in %s:\n' "$versions"
for folder in "$versions"/*; do
    [ -d "$folder" ] && [ ! -L "$folder" ] || continue
    [ -f "$folder/baley" ] && [ ! -L "$folder/baley" ] || continue
    present=${folder##*/}
    valid_version "$present" || continue
    printf '  %s\n' "$present"
done
