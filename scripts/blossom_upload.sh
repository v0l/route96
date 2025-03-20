#!/bin/bash
# Blossom Image Upload Script
# This script uploads an image to a Blossom server using NIP-24242 authentication

# Exit on error
set -e

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

# Current time and expiration (10 seconds from now)
NOW=$(date +%s)
EXPIRATION=$((NOW + 10))

echo "Preparing to upload file: $FILE_PATH"
echo "File hash: $FILE_HASH"
echo "File size: $FILE_SIZE bytes"
echo "File type: $FILE_TYPE"
echo "Group ID: $GROUP_ID"

# Generate the authentication event
echo "Generating authentication event..."
BASE64_AUTH_EVENT=$(nak event \
    --content='' \
    --kind 24242 \
    -t method='PUT' \
    -t u="${SERVER_URL}/upload" \
    -t t='upload' \
    -t expiration="$EXPIRATION" \
    -t x="$FILE_HASH" \
    -t h="$GROUP_ID" \
    --sec "$SECRET_KEY" | base64)

echo "Authentication event generated"

# Upload the file
echo "Uploading file to ${SERVER_URL}/upload..."

# Create temporary files for response body and headers
TEMP_RESPONSE_FILE=$(mktemp)
TEMP_HEADERS_FILE=$(mktemp)

# Perform the upload and capture HTTP status code, saving headers and body
HTTP_CODE=$(curl -s -w "%{http_code}" \
    -D "$TEMP_HEADERS_FILE" \
    -o "$TEMP_RESPONSE_FILE" \
    "${SERVER_URL}/upload" \
    -X PUT \
    -H "Content-Type: $FILE_TYPE" \
    -H "X-Content-Type: $FILE_TYPE" \
    -H "X-SHA-256: $FILE_HASH" \
    -H "X-Content-Length: $FILE_SIZE" \
    -H "Authorization: Nostr $BASE64_AUTH_EVENT" \
    --data-binary @"$FILE_PATH")

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

echo "Upload successful (HTTP $HTTP_CODE)"
echo "Server response:"
echo "$RESPONSE" | jq . 2>/dev/null || echo "$RESPONSE"

# Extract and display the URL if the response is JSON
if command -v jq &> /dev/null; then
    URL=$(echo "$RESPONSE" | jq -r '.url' 2>/dev/null)
    if [ "$URL" != "null" ] && [ "$URL" != "" ]; then
        echo "File is available at: $URL"
    fi
fi

# Clean up temp files
rm -f "$TEMP_RESPONSE_FILE" "$TEMP_HEADERS_FILE"