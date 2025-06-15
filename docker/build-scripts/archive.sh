set -eu

TARGETS="$(/usr/bin/ls -1 /usr/local/bin/* | tr '\n' ' ' )"
TARGETS="$TARGETS /etc/mirakc/strings.yml"

for EXE in $(/usr/bin/ls -1 /usr/local/bin/*)
do
  LIBS=$(ldd $EXE 2>/dev/null | \
         sed 's/^\s*//' | \
         grep -E -v ^/ | \
         grep '=>' | \
         awk '{print $3}' | \
         tr '\n' ' ')
  if [ -n "$LIBS" ]
  then
    TARGETS="$TARGETS $LIBS"
  fi
  ELF_LOADER=$(ldd $EXE 2>/dev/null | \
               sed 's/^\s*//' | \
               grep -E ^/ | \
               awk '{print $1}' | \
               tr '\n' ' ')
  if [ -n "$ELF_LOADER" ]
  then
    TARGETS="$TARGETS $ELF_LOADER"
  fi
done

TARGETS=$(echo "$TARGETS" | tr ' ' '\n' | sort | uniq | tr '\n' ' ')

tar cvhzf - $TARGETS
