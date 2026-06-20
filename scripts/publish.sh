#!/usr/bin/env sh
# =============================================================================
# rust-ef crates.io publishing script
#
# Publishes 5 crates in dependency order:
#   1. rust-ef-macros                (leaf — no workspace deps)
#   2. rust-ef                       (depends on rust-ef-macros)
#   3. rust-ef-postgres              (depends on rust-ef)
#   4. rust-ef-mysql                 (depends on rust-ef)
#   5. rust-ef-sqlite                (depends on rust-ef)
#
# Usage:
#   sh scripts/publish.sh                 # Dry-run: check + test + package
#   sh scripts/publish.sh --execute       # Real publish to crates.io
#   sh scripts/publish.sh --allow-dirty   # Skip git clean check
#
# Note: crates.io resolves path+version deps against the registry,
# so publishing must happen in strict order with index-update delays.
# The dry-run packages rust-ef-macros only (it has no workspace deps).
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
    printf "  This will publish 5 crates to crates.io. Continue? [y/N] "
    read -r CONFIRM
    case $CONFIRM in [yY]*) ;; *) echo "  Aborted."; exit 0 ;; esac
    echo ""
fi

# ── Step 1: Verify ──────────────────────────────────────────────────────────

echo "${GREEN}[verify]${NC} Checking workspace compiles..."
cargo check -p rust-ef -p rust-ef-macros -p rust-ef-postgres -p rust-ef-mysql -p rust-ef-sqlite 2>/dev/null
echo "  OK"

echo "${GREEN}[verify]${NC} Running tests..."
cargo test -p rust-ef --quiet 2>/dev/null
echo "  OK"
echo ""

# ── Step 2: Publish leaf crate ──────────────────────────────────────────────

echo "${GREEN}[1/5]${NC} ${YELLOW}rust-ef-macros${NC} (leaf — no workspace deps)"
if $DRY_RUN; then
    cargo publish --dry-run $ALLOW_DIRTY -p rust-ef-macros
else
    cargo publish $ALLOW_DIRTY -p rust-ef-macros
    echo "  Waiting for crates.io index (15s)..."
    sleep 15
fi
echo ""

# ── Step 3: Publish rust-ef ─────────────────────────────────────────────────

echo "${GREEN}[2/5]${NC} ${YELLOW}rust-ef${NC} (needs rust-ef-macros on crates.io)"
if $DRY_RUN; then
    echo "  (skipped — requires rust-ef-macros published first)"
else
    cargo publish $ALLOW_DIRTY -p rust-ef
    echo "  Waiting for crates.io index (15s)..."
    sleep 15
fi
echo ""

# ── Step 4: Publish providers ───────────────────────────────────────────────

for i in 1 2 3; do
    case $i in
        1) crate="rust-ef-postgres" ;;
        2) crate="rust-ef-mysql" ;;
        3) crate="rust-ef-sqlite" ;;
    esac
    STEP=$((2 + i))
    echo "${GREEN}[${STEP}/5]${NC} ${YELLOW}${crate}${NC} (needs rust-ef on crates.io)"
    if $DRY_RUN; then
        echo "  (skipped — requires rust-ef published first)"
    else
        cargo publish $ALLOW_DIRTY -p "$crate"
    fi
    echo ""
done

# ── Done ────────────────────────────────────────────────────────────────────

echo ""
echo "${GREEN}============================================${NC}"
if $DRY_RUN; then
    echo "${GREEN}  Workspace compiles, tests pass, rust-ef-macros packages OK.${NC}"
    echo ""
    echo "  To publish (in dependency order):"
    echo "    sh scripts/publish.sh --execute --allow-dirty"
else
    echo "${GREEN}  All 5 crates published to crates.io!${NC}"
fi
echo "${GREEN}============================================${NC}"
