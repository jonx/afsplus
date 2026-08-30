#!/bin/sh
# SPDX-License-Identifier: BSD-2-Clause

set -eu

settings_dir="$HOME/Library/Group Containers/group.com.apple.fskit.settings"
enabled_plist="$settings_dir/enabledModules.plist"
plist_buddy=/usr/libexec/PlistBuddy
pluginkit=/usr/bin/pluginkit
pkgutil=/usr/sbin/pkgutil
sw_vers=/usr/bin/sw_vers
module_local=io.macfuse.app.fsmodule.macfuse-local
module_generic=io.macfuse.app.fsmodule.macfuse
macfuse_receipt=io.macfuse.installer.components.core

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
    [ -x "$pluginkit" ] || {
        echo "Missing $pluginkit." >&2
        exit 69
    }
}

show_versions() {
    product_version=$($sw_vers -productVersion)
    build_version=$($sw_vers -buildVersion)
    macfuse_version=$($pkgutil --pkg-info "$macfuse_receipt" 2>/dev/null |
        sed -n 's/^version: //p')
    [ -n "$macfuse_version" ] || macfuse_version="not installed"
    echo "macOS: $product_version ($build_version)"
    echo "macFUSE: $macfuse_version"
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

has_registered_module() {
    module=$1
    $pluginkit -m -A -D -i "$module" 2>/dev/null | grep -Fq "$module"
}

check_registered_modules() {
    missing=0
    for module in "$module_local" "$module_generic"; do
        if has_registered_module "$module"; then
            echo "installed: $module"
        else
            echo "not installed: $module"
            missing=1
        fi
    done
    return "$missing"
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
    echo "Restarting fskitd requires one administrator authorization."
    if ! sudo /usr/bin/killall fskitd; then
        echo "fskitd was not restarted; no activation change is being kept." >&2
        return 1
    fi
    killall fskit_agent 2>/dev/null || true
    killall extensionkitservice 2>/dev/null || true
}

timestamped_backup() {
    stamp=$(date '+%Y%m%d-%H%M%S')
    echo "$enabled_plist.afsplus-backup-$stamp"
}

enable_modules() {
    check_registered_modules || {
        echo "Install macFUSE and open File System Extensions once before retrying." >&2
        exit 69
    }
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
    if ! restart_fskit; then
        cp -p "$backup" "$enabled_plist"
        validate_plist "$enabled_plist"
        echo "Restored the original module list: $backup" >&2
        exit 77
    fi
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
    if ! restart_fskit; then
        cp -p "$backup" "$enabled_plist"
        validate_plist "$enabled_plist"
        echo "Restored the module list that was current before this command." >&2
        exit 77
    fi
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
        show_versions
        registration_status=0
        check_registered_modules || registration_status=$?
        enabled_status=0
        check_modules "$enabled_plist" || enabled_status=$?
        [ "$registration_status" -eq 0 ] && [ "$enabled_status" -eq 0 ]
        ;;
    enable)
        [ "$#" -eq 0 ] || usage
        show_versions
        enable_modules
        ;;
    restore)
        show_versions
        restore_modules "$@"
        ;;
    *)
        usage
        ;;
esac
