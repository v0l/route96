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

echo "Preparing to download file with hash: $FILE_HASH"
echo "From server: $SERVER_URL"
echo "Group ID: $GROUP_ID"

# Generate the authentication event
echo "Generating authentication event..."
BASE64_AUTH_EVENT=$(nak event \
    --content='' \
    --kind 24242 \
    -t method='GET' \
    -t u="${SERVER_URL}/${FILE_HASH}" \
    -t t='get' \
    -t expiration="$EXPIRATION" \
    -t x="$FILE_HASH" \
    -t h="$GROUP_ID" \
    --sec "$SECRET_KEY" | base64)

echo "Authentication event generated"

# Determine output file name if not provided
if [ -z "$OUTPUT_FILE" ]; then
    # First try to get file info to determine extension
    echo "Getting file information..."
    FILE_INFO=$(curl -s -I "${SERVER_URL}/${FILE_HASH}" \
        -H "Authorization: Nostr $BASE64_AUTH_EVENT")

    # Try to extract content type
    CONTENT_TYPE=$(echo "$FILE_INFO" | grep -i "Content-Type:" | sed 's/Content-Type: *//i' | tr -d '\r')

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
    else
        EXT=""
    fi

    OUTPUT_FILE="./${FILE_HASH}${EXT}"
fi

echo "Downloading file to: $OUTPUT_FILE"

# Download the file to a temporary location first
TEMP_FILE=$(mktemp)

# First check if we can access the file
HTTP_CODE=$(curl -s -w "%{http_code}" -o /dev/null "${SERVER_URL}/${FILE_HASH}" \
    -H "Authorization: Nostr $BASE64_AUTH_EVENT")

if [ "$HTTP_CODE" -lt 200 ] || [ "$HTTP_CODE" -ge 300 ]; then
    echo "Error: Server returned HTTP $HTTP_CODE"
    rm -f "$TEMP_FILE"
    exit 1
fi

# If we get here, we have a valid response, proceed with download
if ! curl -s "${SERVER_URL}/${FILE_HASH}" \
    -H "Authorization: Nostr $BASE64_AUTH_EVENT" \
    --output "$TEMP_FILE"; then
    echo "Error: Failed to download file"
    rm -f "$TEMP_FILE"
    exit 1
fi

# Check if download was successful and move to final location
if [ -f "$TEMP_FILE" ]; then
    FILE_SIZE=$(stat -f%z "$TEMP_FILE" 2>/dev/null || stat -c%s "$TEMP_FILE")
    mv "$TEMP_FILE" "$OUTPUT_FILE"
    echo "Download complete!"
    echo "File saved to: $OUTPUT_FILE"
    echo "File size: $FILE_SIZE bytes"
else
    echo "Error: Failed to download file"
    rm -f "$TEMP_FILE"
    exit 1
fi