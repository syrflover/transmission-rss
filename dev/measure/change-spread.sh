#!/usr/bin/env bash
# How far one change reaches: per non-merge feat, fix, refactor or perf commit,
# the files and crates it touched (the refactor's yardstick, tickets 0092 and
# 0114).
#
#   dev/measure/change-spread.sh <first day> <last day> [<rev>]
#
# Days are author dates (YYYY-MM-DD), both included; <rev> defaults to HEAD.
# Prints one line per commit on stderr and the averages on stdout. A file is any changed text
# file outside docs/ and *.md; a crate is crates/<name>/, and everything under
# web/ counts as one crate. Commits that touch no crate count as 0 crates.
# This is the method of the structure survey of 2026-10-07 (the refactoring
# spec's record), which reported 3.05 crates and 15.6 files for 2026-10-04–07.
set -euo pipefail
[[ $# -ge 2 ]] || { sed -n '6p' "$0" | sed 's/^# \{0,1\}//'; exit 2; }
first=$1 last=$2 rev=${3:-HEAD}

git log --no-merges --format='%H %ad %s' --date=short "$rev" |
  awk -v a="$first" -v b="$last" '$2>=a && $2<=b' |
  grep -E '^[0-9a-f]+ [0-9-]+ (feat|fix|refactor|perf)(\(|!|:)' |
  while read -r h d s; do
    files=0 crates=""
    while IFS=$'\t' read -r added _ path; do
      [[ $added == - ]] && continue
      case $path in docs/* | *.md) continue ;; esac
      files=$((files + 1))
      c=$(sed -nE 's#^crates/([^/]+)/.*#\1#p; s#^(web)/.*#\1#p' <<<"$path")
      [[ -n $c ]] && crates="$crates $c"
    done < <(git show --numstat --format= "$h")
    n=$(tr ' ' '\n' <<<"$crates" | sort -u | grep -c . || true)
    echo "$d ${h:0:7} files=$files crates=$n ${s:0:70}"
  done |
  tee /dev/stderr |
  awk '{split($3,f,"="); split($4,c,"="); n++; F+=f[2]; C+=c[2]}
       END {if (n) printf "commits=%d avg_crates=%.2f avg_files=%.1f\n", n, C/n, F/n; else print "commits=0"}'
