#!/bin/sh
# Answers as `trss-extract` does when the disk it unpacks to is full
# (tests/it/unpack.rs). A committed file, not one a test writes: a script
# written while another test starts a process can fail with "text file busy".
echo '{"failed":"압축을 풀 자리에 쓰지 못했어요: No space left on device (os error 28)"}'
