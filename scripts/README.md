# Route96 Dynamic Whitelist Scripts

This directory contains scripts and examples for the Route96 dynamic whitelist system.

## Overview

The dynamic whitelist system allows Route96 to call an external program to determine whether a user (identified by their Nostr pubkey) should be allowed access to the server. This provides much more flexibility than the static whitelist configuration.

## How It Works

1. When a user tries to access Route96, the server calls the configured external program
2. The program receives the user's pubkey as a command-line argument
3. The program should return:
   - Exit code 0: Access granted
   - Exit code 1: Access denied
   - Exit code 2: Error (treated as denied)
4. Positive responses are cached for the configured duration (default: 1 hour)

## Configuration

Add the following to your `config.yaml`:

```yaml
dynamic_whitelist:
  user_exit_program: "./scripts/check_access.sh"
  cache_duration_seconds: 3600  # Optional, default: 3600 (1 hour)
```

## Sample Script

The `check_access.sh` script demonstrates several ways to implement access control:

### 1. Simple Array Check
Maintain a list of allowed pubkeys directly in the script.

### 2. File-based Check
Read allowed pubkeys from a file (see `allowed_pubkeys.txt`).

### 3. Database Check
Query a database to check if the pubkey is allowed.

### 4. API Call
Make an HTTP request to an external authentication service.

### 5. Time-based Access
Implement time-based access control (e.g., business hours only).

### 6. Rate Limiting
Implement simple rate limiting by checking access logs.

## Creating Your Own Script

Your script should:

1. Accept exactly one argument (the pubkey)
2. Validate the pubkey format (64-character hex string)
3. Implement your access control logic
4. Return appropriate exit codes
5. Output helpful messages to stdout/stderr for logging

### Example Template

```bash
#!/bin/bash

# Check if pubkey argument is provided
if [ $# -ne 1 ]; then
    echo "Usage: $0 <pubkey>" >&2
    exit 2
fi

PUBKEY="$1"

# Validate pubkey format
if [[ ! $PUBKEY =~ ^[0-9a-fA-F]{64}$ ]]; then
    echo "Invalid pubkey format: $PUBKEY" >&2
    exit 2
fi

# Your access control logic here
# ...

# Default: Access denied
echo "Access denied for pubkey: $PUBKEY" >&2
exit 1
```

## Security Considerations

- Make sure your script is executable and has appropriate permissions
- Consider the security implications of your access control logic
- Be careful with database queries and API calls to avoid injection attacks
- Log access attempts for audit purposes
- Consider implementing rate limiting to prevent abuse

## Testing

You can test your script manually:

```bash
# Test with a valid pubkey
./scripts/check_access.sh 63fe6318dc58583cfe16810f86dd09e18bfd76aabc24a0081ce2856f330504ed

# Test with an invalid pubkey
./scripts/check_access.sh 1234567890abcdef

# Check exit code
echo $?
```

## Migration from Static Whitelist

If you're migrating from the static whitelist, you can:

1. Keep the static whitelist as a fallback
2. Create a script that reads from the same list
3. Gradually implement more sophisticated access control

The dynamic whitelist takes precedence over the static whitelist when both are configured. 