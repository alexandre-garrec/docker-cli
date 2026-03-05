#!/bin/bash
mkdir -p docs
ABS_PWD=$(pwd)
BINARY="$ABS_PWD/target/debug/docker-cli"

capture_terminal() {
    local view=$1
    local output=$2
    local delay=$3
    echo "Processing $view -> $output (waiting ${delay}s via Terminal.app)..."
    
    # Set the backdoor file
    if [ "$view" == "none" ]; then
        rm -f screenshot_backdoor.txt
    else
        echo "$view" > screenshot_backdoor.txt
    fi
    
    # Launch Terminal.app
    osascript <<EOF
    tell application "Terminal"
        activate
        do script "cd $ABS_PWD && $BINARY"
        delay $delay
        
        set winID to id of front window
        do shell script "screencapture -l " & winID & " -x $output"
        
        delay 1
        
        -- Send "q" to quit the application cleanly
        tell application "System Events"
            keystroke "q"
        end tell
        
        delay 2
        
        -- Close the window
        tell front window to close
    end tell
EOF
    sleep 3
}

# 30 seconds for every view to be absolutely sure
capture_terminal "none" "docs/screenshot-main.png" 30
capture_terminal "health" "docs/screenshot-health.png" 30
capture_terminal "images" "docs/screenshot-images.png" 30
capture_terminal "volumes" "docs/screenshot-volumes.png" 30
capture_terminal "networks" "docs/screenshot-networks.png" 30

# Cleanup
rm -f screenshot_backdoor.txt

echo "-----------------------------------"
echo "VERIFICATION:"
ls -lh docs/screenshot-*.png
echo "-----------------------------------"
shasum docs/screenshot-*.png
echo "-----------------------------------"
