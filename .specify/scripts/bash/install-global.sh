#!/bin/bash

# =============================================================================
# Install Specify CLI Globally Script
# =============================================================================
# This script installs the Specify CLI package in development mode,
# making the 'specify' command available globally on your system.
# 
# Supports both pip and uv (recommended for faster installation)
# =============================================================================

set -e

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
CYAN='\033[0;36m'
NC='\033[0m' # No Color

# Get the directory where this script is located
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# Go up two levels: from scripts/bash/ to the repo root
REPO_ROOT="$(cd "$SCRIPT_DIR" && cd ../.. && pwd)"

echo -e "${BLUE}========================================${NC}"
echo -e "${BLUE}  Specify CLI Global Installation${NC}"
echo -e "${BLUE}========================================${NC}"
echo ""

# Check for uv (recommended)
USE_UV=false
if command -v uv &> /dev/null; then
    USE_UV=true
    echo -e "${CYAN}✓${NC} Found uv (recommended for fast installation)"
fi

# Check if Python is available
if ! command -v python3 &> /dev/null && ! $USE_UV; then
    echo -e "${RED}Error: Python 3 is not installed or not in PATH${NC}"
    echo "Please install Python 3.11 or later from https://python.org/"
    exit 1
fi

# Check Python version
PYTHON_VERSION=$(python3 -c 'import sys; print(".".join(map(str, sys.version_info[:2])))')
REQUIRED_VERSION="3.11"

if [ "$(printf '%s\n' "$REQUIRED_VERSION" "$PYTHON_VERSION" | sort -V | head -n1)" != "$REQUIRED_VERSION" ]; then
    if $USE_UV; then
        echo -e "${YELLOW}Python version $PYTHON_VERSION is old. Using uv to install Python 3.11...${NC}"
        uv python install 3.11
    else
        echo -e "${RED}Error: Python $PYTHON_VERSION is too old. Requires Python $REQUIRED_VERSION or later.${NC}"
        echo ""
        echo "Options:"
        echo "  1. Install Python 3.11+ from https://python.org/"
        echo "  2. Install uv for automatic Python management: https://github.com/astral-sh/uv"
        exit 1
    fi
fi

echo -e "${GREEN}✓${NC} Python version: $PYTHON_VERSION"

# Navigate to repository root
cd "$REPO_ROOT"

echo ""
echo -e "${YELLOW}Installing Specify CLI in development mode...${NC}"
echo ""

# Install the package
if $USE_UV; then
    echo -e "${CYAN}Using uv to install...${NC}"
    # Use uv with the installed Python 3.11 (globally with break-system-packages)
    uv pip install -e . --python 3.11 --system --break-system-packages
else
    # Check if pip is available
    if ! command -v pip3 &> /dev/null && ! python3 -m pip --version &> /dev/null; then
        echo -e "${RED}Error: pip is not installed${NC}"
        exit 1
    fi
    echo -e "Using pip to install..."
    python3 -m pip install -e . --quiet
fi

# Check if installation was successful
if command -v specify &> /dev/null; then
    echo ""
    echo -e "${GREEN}========================================${NC}"
    echo -e "${GREEN}  Installation Successful!${NC}"
    echo -e "${GREEN}========================================${NC}"
    echo ""
    echo -e "You can now use the ${BLUE}specify${NC} command from anywhere!"
    echo ""
    echo "Try running:"
    echo "  ${YELLOW}specify --help${NC}"
    echo ""
    echo "Or initialize a new project:"
    echo "  ${YELLOW}specify init${NC}"
    echo ""
else
    # Check where uv installed specify
    UV_BIN_DIR=$(uv python show --python 3.11 2>/dev/null | grep "bin" || echo "")
    echo -e "${YELLOW}Note: You may need to add the specify CLI to your PATH:${NC}"
    echo ""
    if [ -n "$UV_BIN_DIR" ]; then
        echo "Add this to your ~/.bashrc or ~/.zshrc:"
        echo "  export PATH=\"$UV_BIN_DIR:\$PATH\""
    else
        echo "Add this to your ~/.bashrc or ~/.zshrc:"
        echo "  export PATH=\"\$HOME/.local/share/uv/python/cpython-3.11.15-macos-x86_64-none/bin:\$PATH\""
    fi
    echo ""
    echo "Or restart your terminal and run:"
    echo "  ${YELLOW}specify --help${NC}"
fi
