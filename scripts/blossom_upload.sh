#!/bin/bash
# Blossom Image Upload Script
# This script uploads an image to a Blossom server using NIP-24242 authentication

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
    echo "Usage: $0 <image_file> <server_url> <group_id> <secret_key>"
    echo "Example: $0 image.jpg http://example.com your-group-id npub1..."
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

# Generate the authentication event
BASE64_AUTH_EVENT=$(nak event \
    --content='Upload file' \
    --kind 24242 \
    -t t='upload' \
    -t expiration="$EXPIRATION" \
    -t x="$FILE_HASH" \
    -t h="$GROUP_ID" \
    --sec "$SECRET_KEY" | base64)

# Upload the file

# Create temporary files for response body and headers
TEMP_RESPONSE_FILE=$(mktemp)
TEMP_HEADERS_FILE=$(mktemp)

# If FILE_TYPE is text/plain for a markdown file, correct it
if [[ "$FILE_PATH" == *.md ]] && [[ "$FILE_TYPE" == "text/plain" ]]; then
    FILE_TYPE="text/markdown"
fi

# Ensure consistent content types between headers

# Perform the upload and capture HTTP status code, saving headers and body
CURL_COMMAND="curl -s \\
    -D \"$TEMP_HEADERS_FILE\" \\
    -o \"$TEMP_RESPONSE_FILE\" \\
    \"${SERVER_URL}/upload\" \\
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

# Bug fix: The server returns a 404 with "File not found" when a file already exists
# This is a bug in the server's implementation of process_stream() in routes/blossom.rs
if [ "$HTTP_CODE" = "404" ] && [ "$REASON" = "File not found" ]; then
    echo "The server returned 404 with 'File not found', but this actually indicates the file already exists."
    echo "File hash: $FILE_HASH"
    echo "Full response headers:"
    cat "$TEMP_HEADERS_FILE"
    # Clean up temp files
    rm -f "$TEMP_RESPONSE_FILE" "$TEMP_HEADERS_FILE"
    # Special exit code for this case
    exit 2
fi

# Check for 500 error with "already exists" in response
if [ "$HTTP_CODE" = "500" ] && [[ "$RESPONSE" == *"already exists"* ]]; then
    echo "Server indicates file already exists (status 500 but with exists message)"
    echo "This is a known issue with some Blossom server versions"
    echo "File hash: $FILE_HASH"
    # Display full headers for debugging
    echo "Full response headers:"
    cat "$TEMP_HEADERS_FILE"
    # Clean up temp files
    rm -f "$TEMP_RESPONSE_FILE" "$TEMP_HEADERS_FILE"
    # Special exit code for this case
    exit 2
fi

# Also handle 409 Conflict - file already exists
if [ "$HTTP_CODE" = "409" ]; then
    echo "Server indicates file already exists (status 409 Conflict)"
    echo "File hash: $FILE_HASH"
    # Clean up temp files
    rm -f "$TEMP_RESPONSE_FILE" "$TEMP_HEADERS_FILE"
    # Special exit code for this case
    exit 2
fi

# Check for error status codes
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

    # Clean up temp files
    rm -f "$TEMP_RESPONSE_FILE" "$TEMP_HEADERS_FILE"
    exit 1
fi

# Extract and display the URL if the response is JSON
if command -v jq &> /dev/null; then
    URL=$(echo "$RESPONSE" | jq -r '.url' 2>/dev/null)
    if [ "$URL" != "null" ] && [ "$URL" != "" ]; then
        :
    fi
fi

# Clean up temp files
rm -f "$TEMP_RESPONSE_FILE" "$TEMP_HEADERS_FILE"