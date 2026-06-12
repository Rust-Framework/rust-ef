#!/usr/bin/env sh
# =============================================================================
# lref crates.io publishing script
#
# Publishes 6 crates in dependency order:
#   1. lref-macros                (leaf — no workspace deps)
#   2. lref                       (depends on lref-macros)
#   3. lref-provider-postgres     (depends on lref)
#   4. lref-provider-mysql        (depends on lref)
#   5. lref-provider-sqlite       (depends on lref)
#   6. lref-cli                   (depends on lref + provider-postgres)
#
# Usage:
#   sh scripts/publish.sh                 # Dry-run: check + test + package
#   sh scripts/publish.sh --execute       # Real publish to crates.io
#   sh scripts/publish.sh --allow-dirty   # Skip git clean check
#
# Note: crates.io resolves path+version deps against the registry,
# so publishing must happen in strict order with index-update delays.
# The dry-run packages lref-macros only (it has no workspace deps).
# =============================================================================

set -e

PROJECT_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$PROJECT_ROOT"

RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m'

DRY_RUN=true
ALLOW_DIRTY=""

for arg in "$@"; do
    case $arg in
        --execute|--publish) DRY_RUN=false ;;
        --dry-run|--check)   DRY_RUN=true ;;
        --allow-dirty)       ALLOW_DIRTY="--allow-dirty" ;;
    esac
done

# ── Git check ───────────────────────────────────────────────────────────────

if [ -z "$ALLOW_DIRTY" ]; then
    DIRTY_FILES=$(git diff-index --name-only HEAD -- 2>/dev/null || true)
    if [ -n "$DIRTY_FILES" ]; then
        printf "${YELLOW}Uncommitted changes:${NC}\n"
        echo "$DIRTY_FILES"
        printf "\n  Continue with --allow-dirty? [y/N] "
        read -r ALLOW
        case $ALLOW in [yY]*) ALLOW_DIRTY="--allow-dirty" ;; *) echo "  Aborted."; exit 0 ;; esac
    fi
fi

# ── Mode banner ─────────────────────────────────────────────────────────────

VERSION=$(grep 'version' Cargo.toml | head -1 | sed 's/.*"\(.*\)"/\1/')

if $DRY_RUN; then
    printf "${YELLOW}=== DRY RUN (v%s) ===${NC}\n" "$VERSION"
    echo "  Verifying workspace compiles, tests pass, and leaf crate packages."
    echo ""
else
    printf "${RED}=== LIVE PUBLISH v%s ===${NC}\n" "$VERSION"
    printf "  This will publish 6 crates to crates.io. Continue? [y/N] "
    read -r CONFIRM
    case $CONFIRM in [yY]*) ;; *) echo "  Aborted."; exit 0 ;; esac
    echo ""
fi

# ── Step 1: Verify ──────────────────────────────────────────────────────────

echo "${GREEN}[verify]${NC} Checking workspace compiles..."
cargo check -p lref -p lref-macros -p lref-provider-postgres -p lref-provider-mysql -p lref-provider-sqlite 2>/dev/null
echo "  OK"

echo "${GREEN}[verify]${NC} Running tests..."
cargo test -p lref --quiet 2>/dev/null
echo "  OK"
echo ""

# ── Step 2: Publish leaf crate ──────────────────────────────────────────────

echo "${GREEN}[1/6]${NC} ${YELLOW}lref-macros${NC} (leaf — no workspace deps)"
if $DRY_RUN; then
    cargo publish --dry-run $ALLOW_DIRTY -p lref-macros
else
    cargo publish $ALLOW_DIRTY -p lref-macros
    echo "  Waiting for crates.io index (15s)..."
    sleep 15
fi
echo ""

# ── Step 3: Publish lref ───────────────────────────────────────────────────

echo "${GREEN}[2/6]${NC} ${YELLOW}lref${NC} (needs lref-macros on crates.io)"
if $DRY_RUN; then
    echo "  (skipped — requires lref-macros published first)"
else
    cargo publish $ALLOW_DIRTY -p lref
    echo "  Waiting for crates.io index (15s)..."
    sleep 15
fi
echo ""

# ── Step 4: Publish providers ───────────────────────────────────────────────

for crate in lref-provider-postgres lref-provider-mysql lref-provider-sqlite; do
    NUM=$(echo "lref-provider-postgres lref-provider-mysql lref-provider-sqlite" | tr ' ' '\n' | grep -n "$crate" | cut -d: -f1)
    STEP=$((2 + NUM))
    echo "${GREEN}[${STEP}/6]${NC} ${YELLOW}${crate}${NC} (needs lref on crates.io)"
    if $DRY_RUN; then
        echo "  (skipped — requires lref published first)"
    else
        cargo publish $ALLOW_DIRTY -p "$crate"
    fi
    echo ""
done

# ── Step 5: Publish cli ─────────────────────────────────────────────────────

echo "${GREEN}[6/6]${NC} ${YELLOW}lref-cli${NC} (needs lref + provider-postgres on crates.io)"
if $DRY_RUN; then
    echo "  (skipped — requires lref published first)"
else
    cargo publish $ALLOW_DIRTY -p lref-cli
fi

# ── Done ────────────────────────────────────────────────────────────────────

echo ""
echo "${GREEN}============================================${NC}"
if $DRY_RUN; then
    echo "${GREEN}  Workspace compiles, tests pass, lref-macros packages OK.${NC}"
    echo ""
    echo "  To publish (in dependency order):"
    echo "    sh scripts/publish.sh --execute --allow-dirty"
else
    echo "${GREEN}  All crates published to crates.io!${NC}"
fi
echo "${GREEN}============================================${NC}"
