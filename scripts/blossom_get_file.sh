#!/bin/bash
# Blossom Get File Script
# This script downloads a file from a Blossom server using NIP-24242 authentication

# Exit on error
set -e

# Check for required arguments
if [ "$#" -lt 4 ]; then
    echo "Usage: $0 <file_hash> <server_url> <group_id> <secret_key> [output_file]"
    echo "Example: $0 abcdef1234567890 http://example.com your-group-id nsec1... ./downloaded_file.jpg"
    exit 1
fi

# Assign arguments to variables
FILE_HASH="$1"
SERVER_URL="$2"
GROUP_ID="$3"
SECRET_KEY="$4"
OUTPUT_FILE="$5"

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
EXPIRATION=$((NOW + 30))

# Define file URL (direct path)
FILE_URL="${SERVER_URL}/${FILE_HASH}"

# Generate the authentication event
BASE64_AUTH_EVENT=$(nak event \
    --content='Get file' \
    --kind 24242 \
    -t t='get' \
    -t expiration="$EXPIRATION" \
    -t x="$FILE_HASH" \
    -t h="$GROUP_ID" \
    --sec "$SECRET_KEY" | base64)

# Determine output file name if not provided
if [ -z "$OUTPUT_FILE" ]; then
    # First try to get file info to determine extension
    # Create temporary headers file
    TEMP_HEADERS_FILE=$(mktemp)

    # Get headers
    curl -s -I "${FILE_URL}" \
        -H "Authorization: Nostr $BASE64_AUTH_EVENT" \
        -D "$TEMP_HEADERS_FILE"

    # Try to extract content type
    CONTENT_TYPE=$(grep -i "Content-Type:" "$TEMP_HEADERS_FILE" | sed 's/Content-Type: *//i' | tr -d '\r')

    # Clean up temporary file
    rm -f "$TEMP_HEADERS_FILE"

    # Determine extension based on content type
    if [[ "$CONTENT_TYPE" == *"image/jpeg"* ]]; then
        EXT=".jpg"
    elif [[ "$CONTENT_TYPE" == *"image/png"* ]]; then
        EXT=".png"
    elif [[ "$CONTENT_TYPE" == *"image/gif"* ]]; then
        EXT=".gif"
    elif [[ "$CONTENT_TYPE" == *"image/webp"* ]]; then
        EXT=".webp"
    elif [[ "$CONTENT_TYPE" == *"video/mp4"* ]]; then
        EXT=".mp4"
    elif [[ "$CONTENT_TYPE" == *"audio/mpeg"* ]]; then
        EXT=".mp3"
    elif [[ "$CONTENT_TYPE" == *"application/pdf"* ]]; then
        EXT=".pdf"
    elif [[ "$CONTENT_TYPE" == *"text/plain"* ]]; then
        EXT=".txt"
    elif [[ "$CONTENT_TYPE" == *"text/markdown"* ]]; then
        EXT=".md"
    else
        EXT=""
    fi

    OUTPUT_FILE="./${FILE_HASH}${EXT}"
fi

# Download the file to a temporary location first
TEMP_FILE=$(mktemp)
TEMP_HEADERS_FILE=$(mktemp)

# First check if we can access the file
curl -s -I "${FILE_URL}" \
    -H "Authorization: Nostr $BASE64_AUTH_EVENT" \
    -D "$TEMP_HEADERS_FILE"

# Get the HTTP code
HTTP_CODE=$(head -1 "$TEMP_HEADERS_FILE" | cut -d' ' -f2)

# Check for specific error cases
if [ "$HTTP_CODE" = "404" ]; then
    echo "Error: File not found (HTTP 404)"
    rm -f "$TEMP_FILE" "$TEMP_HEADERS_FILE"
    exit 1
fi

if [ "$HTTP_CODE" = "403" ]; then
    echo "Error: Access denied (HTTP 403)"
    echo "This could be because:"
    echo "1. The file belongs to another user"
    echo "2. The file was uploaded with a different group ID"
    echo "3. The authentication credentials are incorrect"
    rm -f "$TEMP_FILE" "$TEMP_HEADERS_FILE"
    exit 1
fi

if [ "$HTTP_CODE" = "500" ]; then
    echo "Error: Server internal error (HTTP 500)"
    echo "This could be due to database connectivity issues or filesystem problems."
    rm -f "$TEMP_FILE" "$TEMP_HEADERS_FILE"
    exit 1
fi

if [ -z "$HTTP_CODE" ] || [ "$HTTP_CODE" -lt 200 ] || [ "$HTTP_CODE" -ge 300 ]; then
    echo "Error: Server returned HTTP $HTTP_CODE"
    rm -f "$TEMP_FILE" "$TEMP_HEADERS_FILE"
    exit 1
fi

# If we get here, we have a valid response, proceed with download
if ! curl -s "${FILE_URL}" \
    -H "Authorization: Nostr $BASE64_AUTH_EVENT" \
    --output "$TEMP_FILE"; then
    echo "Error: Failed to download file"
    rm -f "$TEMP_FILE" "$TEMP_HEADERS_FILE"
    exit 1
fi

# Check if download was successful and move to final location
if [ -f "$TEMP_FILE" ] && [ -s "$TEMP_FILE" ]; then
    FILE_SIZE=$(stat -f%z "$TEMP_FILE" 2>/dev/null || stat -c%s "$TEMP_FILE")

    # Calculate hash of downloaded file to verify it matches
    DOWNLOAD_HASH=$(sha256sum "$TEMP_FILE" | cut -d ' ' -f 1)

    if [ "$DOWNLOAD_HASH" != "$FILE_HASH" ]; then
        echo "Warning: Hash of downloaded file ($DOWNLOAD_HASH) does not match expected hash ($FILE_HASH)"
        echo "This could indicate file corruption or tampering."
        echo "Continuing anyway..."
    else
        echo ""
    fi

    mv "$TEMP_FILE" "$OUTPUT_FILE"
else
    echo "Error: Failed to download file or file is empty"
    rm -f "$TEMP_FILE"
    exit 1
fi

# Clean up
rm -f "$TEMP_HEADERS_FILE"