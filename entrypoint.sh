#!/bin/sh
set -e

./bin/route96 "$@"
EXIT_CODE=$?

if [ $EXIT_CODE -ne 0 ]; then
    case $EXIT_CODE in
        139) echo "CRASH: Process killed by SIGSEGV (segmentation fault)" >&2 ;;
        134) echo "CRASH: Process killed by SIGABRT (abort)" >&2 ;;
        136) echo "CRASH: Process killed by SIGFPE (floating point exception)" >&2 ;;
        137) echo "CRASH: Process killed by SIGKILL (OOM or external kill)" >&2 ;;
        *)   echo "CRASH: Process exited with code $EXIT_CODE" >&2 ;;
    esac
fi

exit $EXIT_CODE
