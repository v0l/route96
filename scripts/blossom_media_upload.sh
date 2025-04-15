#!/bin/bash
# Blossom Media Upload Script (for Optimization/Processing)
# This script uploads a media file to a Blossom server's /media endpoint
# using NIP-24242 authentication. The server is expected to process/optimize this file.

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
    echo "Usage: $0 <media_file> <server_url> <group_id> <secret_key>"
    echo "Example: $0 video.mp4 http://example.com your-group-id npub1..."
    exit 1
fi

# Assign arguments to variables
FILE_PATH="$1"
SERVER_URL="$2"
GROUP_ID="$3"
SECRET_KEY="$4"

# Ensure SERVER_URL does not end with a trailing slash
SERVER_URL=$(echo "$SERVER_URL" | sed 's#/$##')

# Check if file exists
if [ ! -f "$FILE_PATH" ]; then
    echo "Error: File '$FILE_PATH' not found"
    exit 1
fi

# Check if nak is installed
if ! command -v nak &> /dev/null; then
    echo "Error: 'nak' command not found. Please install it first."
    echo "You can install it with: cargo install nak"
    exit 1
fi

# Get file information
FILE_HASH=$(sha256sum "$FILE_PATH" | cut -d ' ' -f 1)
FILE_SIZE=$(stat -f%z "$FILE_PATH" 2>/dev/null || stat -c%s "$FILE_PATH")
FILE_TYPE=$(file --mime-type -b "$FILE_PATH")

# Current time and expiration (30 seconds from now for more reliability)
NOW=$(date +%s)
EXPIRATION=$((NOW + 30))

# Generate the authentication event for the /media endpoint
# Note: t=media is used here instead of t=upload
BASE64_AUTH_EVENT=$(nak event \
    --content='Upload media for processing' \
    --kind 24242 \
    -t t='media' \
    -t expiration="$EXPIRATION" \
    -t x="$FILE_HASH" \
    -t h="$GROUP_ID" \
    --sec "$SECRET_KEY" | base64)

# Upload the file to the /media endpoint

# Create temporary files for response body and headers
TEMP_RESPONSE_FILE=$(mktemp)
TEMP_HEADERS_FILE=$(mktemp)

# If FILE_TYPE is text/plain for a markdown file, correct it (though unlikely for media endpoint)
if [[ "$FILE_PATH" == *.md ]] && [[ "$FILE_TYPE" == "text/plain" ]]; then
    FILE_TYPE="text/markdown"
fi

# Perform the upload and capture HTTP status code, saving headers and body
CURL_COMMAND="curl -s \\
    -D \"$TEMP_HEADERS_FILE\" \\
    -o \"$TEMP_RESPONSE_FILE\" \\
    \"${SERVER_URL}/media\" \\
    -X PUT \\
    -H \"Content-Type: $FILE_TYPE\" \\
    -H \"X-Content-Type: $FILE_TYPE\" \\
    -H \"X-SHA-256: $FILE_HASH\" \\
    -H \"X-Content-Length: $FILE_SIZE\" \\
    -H \"Authorization: Nostr $BASE64_AUTH_EVENT\" \\
    --data-binary @\"$FILE_PATH\""

pause_and_run "$CURL_COMMAND"

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

# Check for error status codes
# Note: The specific "already exists" checks from upload might not apply
#       if /media always returns a new blob descriptor. Kept for robustness.
if [ -z "$HTTP_CODE" ] || [ "$HTTP_CODE" -lt 200 ] || [ "$HTTP_CODE" -ge 300 ]; then
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

    # Display reason if we haven't already
    if [ -n "$REASON" ]; then
        echo "Reason: $REASON"
    fi

    # Display full headers for debugging
    echo "Full response headers:"
    cat "$TEMP_HEADERS_FILE"

    # Clean up temp files
    rm -f "$TEMP_RESPONSE_FILE" "$TEMP_HEADERS_FILE"
    exit 1
fi

echo "Media upload successful (HTTP $HTTP_CODE)."
echo "Response:"
echo "$RESPONSE" | jq . 2>/dev/null || echo "$RESPONSE"
echo "---"
echo "Full response headers:"
cat "$TEMP_HEADERS_FILE"
echo "---"

# Extract and display the URL if the response is JSON
if command -v jq &> /dev/null; then
    URL=$(echo "$RESPONSE" | jq -r '.url' 2>/dev/null)
    SHA256=$(echo "$RESPONSE" | jq -r '.sha256' 2>/dev/null)
    if [ "$URL" != "null" ] && [ "$URL" != "" ]; then
        echo "Processed Media URL: $URL"
    fi
    if [ "$SHA256" != "null" ] && [ "$SHA256" != "" ]; then
        echo "Processed Media SHA256: $SHA256"
        if [ "$SHA256" != "$FILE_HASH" ]; then
            echo "NOTE: Processed SHA256 ($SHA256) differs from original ($FILE_HASH), as expected."
        fi
    fi
fi

# Clean up temp files
rm -f "$TEMP_RESPONSE_FILE" "$TEMP_HEADERS_FILE"

echo "Done."
exit 0