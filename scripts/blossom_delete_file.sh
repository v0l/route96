#!/bin/bash
# Blossom Delete File Script
# This script deletes a file from a Blossom server using NIP-24242 authentication

# Exit on error
set -e

# Check for required arguments
if [ "$#" -lt 4 ]; then
    echo "Usage: $0 <file_hash> <server_url> <group_id> <secret_key>"
    echo "Example: $0 abcdef1234567890 http://example.com your-group-id nsec1..."
    exit 1
fi

# Assign arguments to variables
FILE_HASH="$1"
SERVER_URL="$2"
GROUP_ID="$3"
SECRET_KEY="$4"

# Check if nak is installed
if ! command -v nak &> /dev/null; then
    echo "Error: 'nak' command not found. Please install it first."
    echo "You can install it with: cargo install nak"
    exit 1
fi

# Check if file hash is valid
if ! [[ "$FILE_HASH" =~ ^[0-9a-f]{64}$ ]]; then
    echo "Warning: File hash should be a 64-character hex string"
    echo "Continuing anyway..."
fi

# Current time and expiration (10 seconds from now)
NOW=$(date +%s)
EXPIRATION=$((NOW + 10))

echo "Preparing to delete file with hash: $FILE_HASH"
echo "From server: $SERVER_URL"
echo "Group ID: $GROUP_ID"

# Generate the authentication event
echo "Generating authentication event..."
BASE64_AUTH_EVENT=$(nak event \
    --content='' \
    --kind 24242 \
    -t method='DELETE' \
    -t u="${SERVER_URL}/${FILE_HASH}" \
    -t t='delete' \
    -t expiration="$EXPIRATION" \
    -t x="$FILE_HASH" \
    -t h="$GROUP_ID" \
    --sec "$SECRET_KEY" | base64)

echo "Authentication event generated"

# Delete the file
echo "Deleting file..."

# Create temporary files for response body and headers
TEMP_RESPONSE_FILE=$(mktemp)
TEMP_HEADERS_FILE=$(mktemp)

# Send the delete request and capture HTTP status code, headers, and response body
HTTP_CODE=$(curl -s -w "%{http_code}" \
    -D "$TEMP_HEADERS_FILE" \
    -o "$TEMP_RESPONSE_FILE" \
    "${SERVER_URL}/${FILE_HASH}" \
    -X DELETE \
    -H "Authorization: Nostr $BASE64_AUTH_EVENT")

# Read the response body from the temp file
RESPONSE=$(cat "$TEMP_RESPONSE_FILE")

# Check for error status codes
if [ "$HTTP_CODE" -lt 200 ] || [ "$HTTP_CODE" -ge 300 ]; then
    echo "Error: Server returned HTTP $HTTP_CODE"
    echo "Server response:"
    echo "$RESPONSE" | jq . 2>/dev/null || echo "$RESPONSE"

    # Try to extract error message if it exists
    if command -v jq &> /dev/null; then
        ERROR_MSG=$(echo "$RESPONSE" | jq -r '.message' 2>/dev/null)
        if [ "$ERROR_MSG" != "null" ] && [ "$ERROR_MSG" != "" ]; then
            echo "Error message: $ERROR_MSG"
        fi
    fi

    # Check for X-Reason header in the response
    if grep -q "X-Reason:" "$TEMP_HEADERS_FILE"; then
        REASON=$(grep "X-Reason:" "$TEMP_HEADERS_FILE" | sed 's/X-Reason: //' | tr -d '\r')
        echo "Reason: $REASON"
    fi

    # Clean up temp files
    rm -f "$TEMP_RESPONSE_FILE" "$TEMP_HEADERS_FILE"
    exit 1
fi

# Clean up temp files
rm -f "$TEMP_RESPONSE_FILE" "$TEMP_HEADERS_FILE"

echo "Delete successful (HTTP $HTTP_CODE)"
echo "Server response:"
echo "$RESPONSE" | jq . 2>/dev/null || echo "$RESPONSE"
