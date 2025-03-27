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

# Ensure SERVER_URL does not end with a trailing slash
SERVER_URL=$(echo "$SERVER_URL" | sed 's#/$##')

# Check if pubkey is in the correct format (hex, 64 characters)
if ! [[ "$PUBKEY" =~ ^[0-9a-f]{64}$ ]]; then
    echo "Warning: Pubkey is not a 64-character hex string"
    echo "If you have an npub, convert it to hex first using: nak decode <npub>"
    echo "Continuing anyway..."
fi

# Create temporary files for response body and headers
TEMP_RESPONSE_FILE=$(mktemp)
TEMP_HEADERS_FILE=$(mktemp)

# Make the request to list files
curl -s \
    -D "$TEMP_HEADERS_FILE" \
    -o "$TEMP_RESPONSE_FILE" \
    "${SERVER_URL}/list/${PUBKEY}"

# Get the HTTP code in a macOS-compatible way
HTTP_CODE=$(head -1 "$TEMP_HEADERS_FILE" | cut -d' ' -f2)

# Read the response body from the temp file
RESPONSE=$(cat "$TEMP_RESPONSE_FILE")

# Extract reason header if present
REASON=""
if grep -q "X-Reason:" "$TEMP_HEADERS_FILE"; then
    REASON=$(grep "X-Reason:" "$TEMP_HEADERS_FILE" | sed 's/X-Reason: //' | tr -d '\r')
elif grep -q "x-reason:" "$TEMP_HEADERS_FILE"; then
    REASON=$(grep "x-reason:" "$TEMP_HEADERS_FILE" | sed 's/x-reason: //' | tr -d '\r')
fi

# Special case: Check for timestamp type mismatch errors
if [[ "$REASON" == *"mismatched types"* && "$REASON" == *"TIMESTAMPTZ"* && "$REASON" == *"TIMESTAMP"* ]]; then
    echo "Database schema timestamp error detected. This is a known issue with the Blossom server."
    echo "Full response headers:"
    cat "$TEMP_HEADERS_FILE"
    # Clean up temp files
    rm -f "$TEMP_RESPONSE_FILE" "$TEMP_HEADERS_FILE"
    # Return special exit code for database schema issues
    exit 2
fi

# Check for error status codes
if [ -z "$HTTP_CODE" ] || [ "$HTTP_CODE" -lt 200 ] || [ "$HTTP_CODE" -ge 300 ]; then
    echo "Error: Server returned HTTP $HTTP_CODE"
    echo "Server response:"
    echo "$RESPONSE"

    # Display reason if we haven't already
    if [ -n "$REASON" ]; then
        echo "Reason: $REASON"
    fi

    # Clean up temp files
    rm -f "$TEMP_RESPONSE_FILE" "$TEMP_HEADERS_FILE"
    exit 1
fi

# Check if the response is valid JSON
if ! echo "$RESPONSE" | jq . &>/dev/null; then
    echo "Error: Invalid response from server (not valid JSON)"
    echo "Response: $RESPONSE"
    rm -f "$TEMP_RESPONSE_FILE" "$TEMP_HEADERS_FILE"
    exit 1
fi

# Display the response in a formatted way
echo "Files found (JSON response):"
echo "$RESPONSE" | jq '.'

# Count the number of files
FILE_COUNT=$(echo "$RESPONSE" | jq '. | length')
echo "Total files: $FILE_COUNT"

# Check for files array vs. structured response with files field
if echo "$RESPONSE" | jq -e '.files' &>/dev/null; then
    # If the response has a 'files' field (PagedResult format), output that
    echo "Files listing (from structured response):"
    echo "$RESPONSE" | jq -r '.files[] | "- \(.sha256 // .id // "unknown") | \(.mime_type // .type // "unknown") | \(.size // "unknown") bytes"'
else
    # If the response is a direct array, output that
    echo "Files listing (from direct array):"
    echo "$RESPONSE" | jq -r '.[] | "- \(.sha256 // .id // "unknown") | \(.mime_type // .type // "unknown") | \(.size // "unknown") bytes"'
fi

# Clean up temp files
rm -f "$TEMP_RESPONSE_FILE" "$TEMP_HEADERS_FILE"