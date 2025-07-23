#!/bin/bash

# Route96 Dynamic Whitelist External Program
# This script is called by Route96 to check if a pubkey should be allowed access
#
# Usage: ./check_access.sh <pubkey>
# 
# Return codes:
#   0 = Access granted
#   1 = Access denied
#   2 = Error (will be treated as denied)
#
# The pubkey is passed as the first argument in hexadecimal format
# (e.g., "63fe6318dc58583cfe16810f86dd09e18bfd76aabc24a0081ce2856f330504ed")

set -e

# Check if pubkey argument is provided
if [ $# -ne 1 ]; then
    echo "Usage: $0 <pubkey>" >&2
    exit 2
fi

PUBKEY="$1"

# Validate pubkey format (should be 64 character hex string)
if [[ ! $PUBKEY =~ ^[0-9a-fA-F]{64}$ ]]; then
    echo "Invalid pubkey format: $PUBKEY" >&2
    exit 2
fi

# Example 1: Simple whitelist check
# You can maintain a simple list of allowed pubkeys
ALLOWED_PUBKEYS=(
    "63fe6318dc58583cfe16810f86dd09e18bfd76aabc24a0081ce2856f330504ed"
    "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"
#   "0f4795bf31824a414148daf1b589bb8138fb0a03963f984c84462e40a8365abe"
    # Add more allowed pubkeys here
)

for allowed in "${ALLOWED_PUBKEYS[@]}"; do
    if [ "$PUBKEY" = "$allowed" ]; then
        echo "Access granted for pubkey: $PUBKEY"
        exit 0
    fi
done

# Example 2: Check against a file
# You can also read from a file containing allowed pubkeys
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ALLOWED_PUBKEYS_FILE="$SCRIPT_DIR/allowed_pubkeys.txt"

if [ -f "$ALLOWED_PUBKEYS_FILE" ]; then
    while IFS= read -r line; do
        # Skip comments and empty lines
        [[ $line =~ ^[[:space:]]*# ]] && continue
        [[ -z $line ]] && continue
        echo "Checking pubkey: $line" 
        if [ "$PUBKEY" = "$line" ]; then
            echo "Access granted for pubkey: $PUBKEY (from file)"
            exit 0
        fi
    done < "$ALLOWED_PUBKEYS_FILE"
fi

# Example 3: Database check
# You can query a database to check if the pubkey is allowed
# if command -v mysql >/dev/null 2>&1; then
#     if mysql -u username -ppassword -h localhost -e "SELECT 1 FROM allowed_users WHERE pubkey='$PUBKEY' LIMIT 1;" database_name 2>/dev/null | grep -q "1"; then
#         echo "Access granted for pubkey: $PUBKEY (from database)"
#         exit 0
#     fi
# fi

# Example 4: API call
# You can make an HTTP request to an external service
# if command -v curl >/dev/null 2>&1; then
#     if curl -s -f "https://your-auth-service.com/check?pubkey=$PUBKEY" >/dev/null 2>&1; then
#         echo "Access granted for pubkey: $PUBKEY (from API)"
#         exit 0
#     fi
# fi

# Example 5: Time-based access
# You can implement time-based access control
# CURRENT_HOUR=$(date +%H)
# if [ "$CURRENT_HOUR" -ge 9 ] && [ "$CURRENT_HOUR" -le 17 ]; then
#     echo "Access granted for pubkey: $PUBKEY (within business hours)"
#     exit 0
# fi

# Example 6: Rate limiting
# You can implement simple rate limiting by checking access logs
# ACCESS_COUNT=$(grep -c "$PUBKEY" /var/log/route96_access.log 2>/dev/null || echo "0")
# if [ "$ACCESS_COUNT" -lt 100 ]; then
#     echo "Access granted for pubkey: $PUBKEY (rate limit not exceeded)"
#     exit 0
# fi

# Default: Access denied
echo "Access denied for pubkey: $PUBKEY" >&2
exit 1 