#!/bin/bash
# Blossom Delete File Script
# This script deletes a file from a Blossom server using NIP-24242 authentication

# Exit on error
set -e

# Function to display command and wait for user input
pause_and_run() {
    local command="$1"
    echo "===================================================="
    echo "COMMAND TO RUN:"
    echo "$command"
    echo "===================================================="
    read -p "Press any key to continue..." -n1 -s
    echo ""
    eval "$command"
}

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

# Ensure SERVER_URL does not end with a trailing slash
SERVER_URL=$(echo "$SERVER_URL" | sed 's#/$##')

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

# Current time and expiration (30 seconds from now for more reliability)
NOW=$(date +%s)
EXPIRATION=$((NOW + 120))

# Define the standard delete endpoint URL
DELETE_URL="${SERVER_URL}/${FILE_HASH}"

# Create temporary files for response body and headers
TEMP_RESPONSE_FILE=$(mktemp)
TEMP_HEADERS_FILE=$(mktemp)

# Generate the authentication event for this specific URL
BASE64_AUTH_EVENT=$(nak event \
    --content='' \
    --kind 24242 \
    -t method='DELETE' \
    -t t='delete' \
    -t expiration="$EXPIRATION" \
    -t x="$FILE_HASH" \
    -t h="$GROUP_ID" \
    --sec "$SECRET_KEY" | base64)

# Send the delete request and capture HTTP status code, headers, and response body
CURL_COMMAND="curl -s \\
    -D \"$TEMP_HEADERS_FILE\" \\
    -o \"$TEMP_RESPONSE_FILE\" \\
    \"${DELETE_URL}\" \\
    -X DELETE \\
    -H \"Authorization: Nostr $BASE64_AUTH_EVENT\""

pause_and_run "$CURL_COMMAND"

# Get the HTTP code
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

# Check for success case
if [ "$HTTP_CODE" -ge 200 ] && [ "$HTTP_CODE" -lt 300 ]; then
    rm -f "$TEMP_RESPONSE_FILE" "$TEMP_HEADERS_FILE"
    exit 0
fi

# Check for special cases
if [ "$HTTP_CODE" = "404" ]; then
    if [ "$REASON" = "File not found" ] || [[ "$RESPONSE" == *"File not found"* ]]; then
        echo "File not found at $DELETE_URL"
    fi
elif [ "$HTTP_CODE" = "403" ]; then
    if [[ "$RESPONSE" == *"dont own this file"* ]] || [[ "$RESPONSE" == *"cannot delete"* ]] || [[ "$RESPONSE" == *"forbidden"* ]] || [[ "$RESPONSE" == *"Not authorized"* ]]; then
        echo "⚠️ You don't have permission to delete this file."
        echo "This could be because:"
        echo "1. The file belongs to another user"
        echo "2. The file was uploaded with a different group ID"
        echo "3. The authentication credentials are incorrect"
        echo "4. The server requires admin privileges for deletion"
    fi
elif [ "$HTTP_CODE" = "500" ]; then
    echo "⚠️ Server returned internal error (500) for $DELETE_URL"
    echo "This could be due to database connectivity issues or filesystem problems."
fi

# Clean up temp files
rm -f "$TEMP_RESPONSE_FILE" "$TEMP_HEADERS_FILE"

# If we get here, we didn't succeed
echo "❌ Failed to delete file via $DELETE_URL (HTTP $HTTP_CODE)"
exit 1
