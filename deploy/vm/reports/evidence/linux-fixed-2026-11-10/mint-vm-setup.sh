#!/bin/sh
set -eu
export DEBIAN_FRONTEND=noninteractive
for source in /etc/apt/sources.list /etc/apt/sources.list.d/*.list; do
    [ ! -f "$source" ] || sed -i '/^[[:space:]]*deb.*cdrom:/s/^/#/' "$source"
done
apt-get update
apt-get install -y openssh-server xterm mint-meta-mate
apt-get clean
install -d -m 700 -o tester -g tester /home/tester/.ssh
install -m 600 -o tester -g tester /root/vmctl-authorized-key /home/tester/.ssh/authorized_keys
printf 'tester ALL=(ALL) NOPASSWD: ALL\n' > /etc/sudoers.d/vmctl-tester
chmod 440 /etc/sudoers.d/vmctl-tester
cat > /etc/ssh/sshd_config.d/50-vmctl.conf <<'EOF'
PasswordAuthentication no
KbdInteractiveAuthentication no
EOF
ssh-keygen -A
systemctl enable ssh
mkdir -p /home/tester/.local/bin /home/tester/.config/autostart
cat > /home/tester/.local/bin/vmctl-desktop-init <<'EOF'
#!/bin/sh
systemctl --user import-environment DISPLAY XAUTHORITY XDG_CURRENT_DESKTOP XDG_SESSION_TYPE XDG_SESSION_ID DESKTOP_SESSION
exec xterm -geometry 100x12+50+50 -hold -e sh -c 'ssh-keygen -lf /etc/ssh/ssh_host_ed25519_key.pub; echo VMCTL-MINT-22.3'
EOF
chmod +x /home/tester/.local/bin/vmctl-desktop-init
cat > /home/tester/.config/autostart/vmctl-fingerprint.desktop <<'EOF'
[Desktop Entry]
Type=Application
Name=VM SSH fingerprint
Exec=/home/tester/.local/bin/vmctl-desktop-init
EOF
chown -R tester:tester /home/tester/.local /home/tester/.config
