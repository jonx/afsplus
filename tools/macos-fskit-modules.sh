#!/bin/sh

set -eu

settings_dir="$HOME/Library/Group Containers/group.com.apple.fskit.settings"
enabled_plist="$settings_dir/enabledModules.plist"
plist_buddy=/usr/libexec/PlistBuddy
module_local=io.macfuse.app.fsmodule.macfuse-local
module_generic=io.macfuse.app.fsmodule.macfuse

usage() {
    echo "usage: $0 check | enable | restore <backup.plist>" >&2
    exit 64
}

require_macos_tools() {
    [ "$(uname -s)" = Darwin ] || {
        echo "This tool is only supported on macOS." >&2
        exit 69
    }
    [ -x "$plist_buddy" ] || {
        echo "Missing $plist_buddy." >&2
        exit 69
    }
}

validate_plist() {
    candidate=$1
    [ -f "$candidate" ] || {
        echo "Missing FSKit module list: $candidate" >&2
        echo "Open File System Extensions in System Settings once, then retry." >&2
        exit 66
    }
    plutil -lint "$candidate" >/dev/null
    first_line=$($plist_buddy -c Print "$candidate" | sed -n '1p')
    [ "$first_line" = "Array {" ] || {
        echo "Refusing to modify a non-array FSKit module list: $candidate" >&2
        exit 65
    }
}

has_module() {
    candidate=$1
    module=$2
    $plist_buddy -c Print "$candidate" | grep -Fqx "    $module"
}

check_modules() {
    candidate=$1
    missing=0
    for module in "$module_local" "$module_generic"; do
        if has_module "$candidate" "$module"; then
            echo "enabled: $module"
        else
            echo "missing: $module"
            missing=1
        fi
    done
    return "$missing"
}

restart_fskit() {
    killall fskit_agent 2>/dev/null || true
    killall extensionkitservice 2>/dev/null || true
    echo "Restarting fskitd requires one administrator authorization."
    sudo /usr/bin/killall fskitd
}

timestamped_backup() {
    stamp=$(date '+%Y%m%d-%H%M%S')
    echo "$enabled_plist.afsplus-backup-$stamp"
}

enable_modules() {
    if check_modules "$enabled_plist"; then
        echo "macFUSE FSKit modules are already enabled; nothing changed."
        return
    fi

    task_dir=$(mktemp -d "${TMPDIR:-/tmp}/afsplus-fskit.XXXXXX")
    trap 'rm -r "$task_dir"' EXIT HUP INT TERM
    staged_plist="$task_dir/enabledModules.plist"
    cp -p "$enabled_plist" "$staged_plist"

    for module in "$module_local" "$module_generic"; do
        if ! has_module "$staged_plist" "$module"; then
            $plist_buddy -c "Add : string $module" "$staged_plist"
        fi
    done
    validate_plist "$staged_plist"

    backup=$(timestamped_backup)
    cp -p "$enabled_plist" "$backup"
    cp -p "$staged_plist" "$enabled_plist"
    echo "Backup: $backup"
    restart_fskit
    check_modules "$enabled_plist"
}

restore_modules() {
    [ "$#" -eq 1 ] || usage
    source_plist=$1
    validate_plist "$source_plist"
    backup=$(timestamped_backup)
    cp -p "$enabled_plist" "$backup"
    cp -p "$source_plist" "$enabled_plist"
    echo "Previous current file saved as: $backup"
    restart_fskit
    check_modules "$enabled_plist" || true
}

require_macos_tools
[ "$#" -ge 1 ] || usage
action=$1
shift
validate_plist "$enabled_plist"

case "$action" in
    check)
        [ "$#" -eq 0 ] || usage
        check_modules "$enabled_plist"
        ;;
    enable)
        [ "$#" -eq 0 ] || usage
        enable_modules
        ;;
    restore)
        restore_modules "$@"
        ;;
    *)
        usage
        ;;
esac
