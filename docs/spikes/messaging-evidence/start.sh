#!/bin/bash
# usage: start.sh NAME [extra claude args]
n=$1; shift
cd /tmp/dovetail-s2
rm -f $n.in; mkfifo $n.in
( exec 3<>$n.in
  claude -p --input-format stream-json --output-format stream-json --verbose \
   --name $n --model haiku --allowedTools "ListAgents,SendMessage,Bash(sleep *)" \
   --settings '{"crossSessionInbound":"accept"}' "$@" <&3 > $n.out 2> $n.err
) &
echo started $n
