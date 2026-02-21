#!/bin/bash
# Create macOS .app bundle and DMG
# Usage: ./scripts/create-macos-bundle.sh [version] [--with-libretranslate]
set -euo pipefail

VERSION="${1:-0.0.0}"
BUNDLE_LT=false
if [[ "${2:-}" == "--with-libretranslate" ]]; then
    BUNDLE_LT=true
fi

APP_NAME="Screen Translate"
BINARY="screen-translate"
APP_DIR="${APP_NAME}.app"
CONTENTS="${APP_DIR}/Contents"
DMG_NAME="ScreenTranslate-${VERSION}-macOS"

echo "=== Building ${APP_NAME} v${VERSION} ==="

# Clean previous build
rm -rf "${APP_DIR}" "${DMG_NAME}.dmg"

# Create .app structure
mkdir -p "${CONTENTS}/MacOS"
mkdir -p "${CONTENTS}/Resources"

# Copy binary
if [[ -f "target/release/${BINARY}" ]]; then
    cp "target/release/${BINARY}" "${CONTENTS}/MacOS/${BINARY}"
elif [[ -f "${BINARY}" ]]; then
    cp "${BINARY}" "${CONTENTS}/MacOS/${BINARY}"
else
    echo "Error: Cannot find ${BINARY} binary"
    exit 1
fi

# Copy Info.plist and inject version
sed "s/VERSION_PLACEHOLDER/${VERSION}/g" macos/Info.plist > "${CONTENTS}/Info.plist"

# Generate .icns from logo.png using sips (available on all macOS)
if [[ -f "assets/logo.png" ]]; then
    echo "Generating AppIcon.icns..."
    ICONSET="AppIcon.iconset"
    mkdir -p "${ICONSET}"
    for SIZE in 16 32 64 128 256 512; do
        sips -z ${SIZE} ${SIZE} "assets/logo.png" --out "${ICONSET}/icon_${SIZE}x${SIZE}.png" >/dev/null 2>&1
    done
    # Retina variants
    for SIZE in 32 64 128 256 512 1024; do
        HALF=$((SIZE / 2))
        sips -z ${SIZE} ${SIZE} "assets/logo.png" --out "${ICONSET}/icon_${HALF}x${HALF}@2x.png" >/dev/null 2>&1
    done
    iconutil -c icns "${ICONSET}" -o "${CONTENTS}/Resources/AppIcon.icns"
    rm -rf "${ICONSET}"
    echo "Icon created."
fi

# Bundle LibreTranslate environment if requested
if [[ "${BUNDLE_LT}" == true ]]; then
    echo "Bundling LibreTranslate environment..."
    if [[ -d "libretranslate" ]]; then
        cp -R "libretranslate" "${CONTENTS}/Resources/libretranslate"
        BUNDLE_LT_DIR="${CONTENTS}/Resources/libretranslate"

        # ── Make the bundled Python venv fully relocatable ──
        # Venvs use symlinks to the system Python which break on other machines.
        # We need to: copy the real binary, copy the stdlib, copy the dylib,
        # and patch the binary so it finds the dylib inside the bundle.

        echo "Making bundled Python relocatable..."

        # 1. Resolve the real Python binary and copy it (replace symlink)
        VENV_PYTHON="${BUNDLE_LT_DIR}/bin/python3"
        if [[ -L "${VENV_PYTHON}" ]] || [[ -L "${BUNDLE_LT_DIR}/bin/python3.12" ]]; then
            REAL_PYTHON=$(readlink -f "${BUNDLE_LT_DIR}/bin/python3")
            PYTHON_VER=$(basename "$(dirname "$(dirname "${REAL_PYTHON}")")")  # e.g. "3.12"
            echo "  Copying real Python binary: ${REAL_PYTHON}"
            rm -f "${BUNDLE_LT_DIR}/bin/python3" "${BUNDLE_LT_DIR}/bin/python3.12" 2>/dev/null
            cp "${REAL_PYTHON}" "${BUNDLE_LT_DIR}/bin/python3"
            ln -sf python3 "${BUNDLE_LT_DIR}/bin/python3.12"
        else
            REAL_PYTHON="${VENV_PYTHON}"
            PYTHON_VER=$("${VENV_PYTHON}" -c "import sys; print(f'{sys.version_info.major}.{sys.version_info.minor}')")
        fi

        # 2. Copy the Python framework dylib and Python.app into the bundle
        #    The framework launcher (bin/python3) execs Resources/Python.app/Contents/MacOS/Python
        #    and both binaries link against the Python dylib. We must:
        #    a) Copy the dylib into lib/
        #    b) Copy Python.app into lib/Resources/ (where the launcher expects it)
        #    c) Patch both binaries to find the dylib via @executable_path
        DYLIB_PATH=$(otool -L "${BUNDLE_LT_DIR}/bin/python3" | awk 'NR==2{print $1}')
        if [[ -n "${DYLIB_PATH}" && "${DYLIB_PATH}" != /usr/lib/* && "${DYLIB_PATH}" != /System/* ]]; then
            # Resolve the real dylib path
            DYLIB_REAL=$(python3 -c "import os; print(os.path.realpath('${DYLIB_PATH}'))" 2>/dev/null || echo "${DYLIB_PATH}")
            DYLIB_NAME=$(basename "${DYLIB_PATH}")
            FRAMEWORK_DIR=$(dirname "${DYLIB_REAL}")

            echo "  Copying Python dylib: ${DYLIB_REAL} -> lib/${DYLIB_NAME}"
            mkdir -p "${BUNDLE_LT_DIR}/lib"
            cp "${DYLIB_REAL}" "${BUNDLE_LT_DIR}/lib/${DYLIB_NAME}"

            # Patch bin/python3 (the launcher) to find dylib relative to itself
            install_name_tool -change "${DYLIB_PATH}" "@executable_path/../lib/${DYLIB_NAME}" "${BUNDLE_LT_DIR}/bin/python3"
            # Set the dylib's install name
            install_name_tool -id "@rpath/${DYLIB_NAME}" "${BUNDLE_LT_DIR}/lib/${DYLIB_NAME}" 2>/dev/null || true

            # Copy Python.app (the real interpreter that the launcher execs)
            if [[ -d "${FRAMEWORK_DIR}/Resources/Python.app" ]]; then
                echo "  Copying Python.app into bundle"
                mkdir -p "${BUNDLE_LT_DIR}/lib/Resources"
                cp -R "${FRAMEWORK_DIR}/Resources/Python.app" "${BUNDLE_LT_DIR}/lib/Resources/Python.app"
                # Patch the real interpreter to find the dylib
                REAL_INTERP="${BUNDLE_LT_DIR}/lib/Resources/Python.app/Contents/MacOS/Python"
                if [[ -f "${REAL_INTERP}" ]]; then
                    install_name_tool -change "${DYLIB_PATH}" "@executable_path/../../../../${DYLIB_NAME}" "${REAL_INTERP}"
                    codesign --force -s - "${REAL_INTERP}" 2>/dev/null || true
                fi
            fi
        fi

        # 3. Copy the Python standard library (venv only has site-packages)
        BASE_PREFIX=$("${REAL_PYTHON}" -c "import sys; print(sys.base_prefix)" 2>/dev/null || echo "")
        if [[ -n "${BASE_PREFIX}" && -d "${BASE_PREFIX}/lib/python${PYTHON_VER}" ]]; then
            STDLIB_SRC="${BASE_PREFIX}/lib/python${PYTHON_VER}"
            STDLIB_DST="${BUNDLE_LT_DIR}/lib/python${PYTHON_VER}"
            echo "  Copying Python stdlib from: ${STDLIB_SRC}"
            # Copy everything except site-packages (we already have our own), test suite, and __pycache__
            rsync -a --exclude='site-packages' --exclude='test' --exclude='tests' \
                     --exclude='__pycache__' --exclude='tkinter' --exclude='idlelib' \
                     --exclude='turtle*' --exclude='ensurepip' \
                     "${STDLIB_SRC}/" "${STDLIB_DST}/"
        fi

        # 4. Fix pyvenv.cfg to be self-contained
        cat > "${BUNDLE_LT_DIR}/pyvenv.cfg" <<PYCFG
home = bin
include-system-site-packages = false
version = ${PYTHON_VER}
PYCFG

        # 5. Re-sign the modified binary (ad-hoc, for Gatekeeper)
        codesign --force -s - "${BUNDLE_LT_DIR}/bin/python3" 2>/dev/null || true
        if [[ -f "${BUNDLE_LT_DIR}/lib/${DYLIB_NAME:-}" ]]; then
            codesign --force -s - "${BUNDLE_LT_DIR}/lib/${DYLIB_NAME}" 2>/dev/null || true
        fi

        echo "  Python relocatable: OK"

        # Fix setuptools for pkg_resources compatibility
        echo "Installing setuptools 67.8.0 for pkg_resources..."
        "${BUNDLE_LT_DIR}/bin/pip" install --quiet 'setuptools==67.8.0' 2>/dev/null || {
            echo "Warning: Failed to install setuptools 67.8.0 (will be handled at build time)"
        }

        echo "LibreTranslate bundled."
    else
        echo "Warning: libretranslate/ not found, skipping bundle"
    fi
fi

# Include LaunchAgent for auto-start option
if [[ -f "macos/com.amaralkaff.screen-translate.plist" ]]; then
    cp "macos/com.amaralkaff.screen-translate.plist" "${CONTENTS}/Resources/"
fi

echo "=== ${APP_DIR} created ==="

# Create DMG with Applications symlink (drag-to-install)
echo "Creating DMG..."

# Unmount any existing DMG with same name
hdiutil detach "/Volumes/${APP_NAME}" 2>/dev/null || true

# Kill lingering diskimage helpers that may hold locks on the DMG file
if lsof "${DMG_NAME}.dmg" >/dev/null 2>&1; then
    echo "DMG file is locked, killing holder processes..."
    lsof -t "${DMG_NAME}.dmg" 2>/dev/null | xargs kill -9 2>/dev/null || true
    sleep 2
fi
rm -f "${DMG_NAME}.dmg"

DMG_STAGING="dmg-staging"
rm -rf "${DMG_STAGING}"
mkdir -p "${DMG_STAGING}"
cp -R "${APP_DIR}" "${DMG_STAGING}/"
ln -s /Applications "${DMG_STAGING}/Applications"

# Give file system a moment to settle
sleep 1

# Retry hdiutil up to 3 times (CI runners can have transient locks)
for attempt in 1 2 3; do
    if hdiutil create -volname "${APP_NAME}" \
        -srcfolder "${DMG_STAGING}" \
        -ov -format UDZO \
        "${DMG_NAME}.dmg"; then
        break
    fi
    if [[ $attempt -eq 3 ]]; then
        echo "Error: hdiutil failed after 3 attempts. Checking for locks..."
        lsof | grep -i "${DMG_NAME}" || true
        exit 1
    fi
    echo "hdiutil attempt $attempt failed, retrying in 3s..."
    lsof -t "${DMG_NAME}.dmg" 2>/dev/null | xargs kill -9 2>/dev/null || true
    sleep 3
done

rm -rf "${DMG_STAGING}"

echo "=== ${DMG_NAME}.dmg created ==="
echo ""
echo "Installation Instructions:"
echo "1. Open the DMG and drag 'Screen Translate.app' to Applications"
echo "2. Grant Input Monitoring permission (System Settings > Privacy & Security)"
if [[ "${BUNDLE_LT}" == true ]]; then
    echo "3. (Optional) Enable auto-start on login:"
    echo "   cp '/Applications/Screen Translate.app/Contents/Resources/com.amaralkaff.screen-translate.plist' \\"
    echo "      ~/Library/LaunchAgents/"
    echo "   launchctl load ~/Library/LaunchAgents/com.amaralkaff.screen-translate.plist"
fi
echo ""
echo "Done."
