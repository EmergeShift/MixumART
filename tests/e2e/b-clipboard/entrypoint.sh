#!/bin/sh
# Starts agentd (which owns /run/mixum/display.sock and the MAP listener) and
# the B compositor as one of its comp clients, then waits.
set -e

mkdir -p /var/lib/mixum /run/mixum
chmod 0711 /var/lib/mixum
# Xwayland 需要这个目录；镜像里不一定有，缺了会报
# "Could not find a free socket for the XServer"。
mkdir -p /tmp/.X11-unix
chmod 1777 /tmp/.X11-unix

# agentd refuses to start when it cannot resolve the task identity (fail closed).
export MIXUM_TASK_USER=root
export XDG_RUNTIME_DIR=/run/mixum
export WAYLAND_DISPLAY=mixum-art-0

mixumd --config /dev/null &

for _ in $(seq 1 100); do
    if [ -f /var/lib/mixum/admin-token ] && [ -S /run/mixum/display.sock ]; then
        break
    fi
    sleep 0.1
done

mixum-env-compositor --map-socket /run/mixum/display.sock &

echo "READY token=$(cat /var/lib/mixum/admin-token)"
wait
