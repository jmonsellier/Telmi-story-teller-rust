launch_storyteller() {
    # telmi-rs : relance storyTeller s'il plante, puis se replie sur celui de Telmi OS.
    log "\n:: Launch Story Teller"
    cd $sysdir
    attempts=0
    while [ ! -f /tmp/.offOrder ] && [ $attempts -lt 3 ]; do
        LD_PRELOAD="$sysdir/lib/libpadsp.so" storyTeller
        status=$?
        sync
        [ -f /tmp/.offOrder ] && break
        attempts=$((attempts + 1))
        log "storyTeller arrete (code $status), tentative $attempts"
        sleep 1
    done
    if [ ! -f /tmp/.offOrder ] && [ -f "$sysdir/bin/storyTeller.telmi" ]; then
        log "repli sur le storyTeller de Telmi OS"
        LD_PRELOAD="$sysdir/lib/libpadsp.so" "$sysdir/bin/storyTeller.telmi"
        sync
    fi
    # Toujours finir par une extinction propre plutot qu'un ecran fige.
    touch /tmp/.offOrder
}
