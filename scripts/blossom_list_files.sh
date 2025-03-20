#!/bin/bash
# Blossom List Files Script
# This script lists files uploaded by a specific pubkey on a Blossom server

# Exit on error
set -e

# Check for required arguments
if [ "$#" -lt 2 ]; then
    echo "Usage: $0 <server_url> <pubkey>"
    echo "Example: $0 http://example.com 79ef92b9ebe6dc1e4ea398f6477f227e95429627b0a33dc89b640e137b256be5"
    exit 1
fi

# Assign arguments to variables
SERVER_URL="$1"
PUBKEY="$2"

# Check if pubkey is in the correct format (hex, 64 characters)
if ! [[ "$PUBKEY" =~ ^[0-9a-f]{64}$ ]]; then
    echo "Error: Pubkey must be a 64-character hex string"
    echo "If you have an npub, convert it to hex first"
    exit 1
fi

echo "Fetching files for pubkey: $PUBKEY"
echo "From server: $SERVER_URL"

# Make the request to list files
RESPONSE=$(curl -s "${SERVER_URL}/list/${PUBKEY}")

# Check if the response is valid JSON
if ! echo "$RESPONSE" | jq . &>/dev/null; then
    echo "Error: Invalid response from server"
    echo "Response: $RESPONSE"
    exit 1
fi

# Display the response in a formatted way
echo "Files found:"
echo "$RESPONSE" | jq -r '.[] | "- \(.sha256) | \(.type // "unknown") | \(.size) bytes | \(.url)"'

# Count the number of files
FILE_COUNT=$(echo "$RESPONSE" | jq '. | length')
echo "Total files: $FILE_COUNT"