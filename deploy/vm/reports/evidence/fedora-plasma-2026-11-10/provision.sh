#!/bin/sh
set -eu
dnf install -y NetworkManager plasma-desktop plasma-workspace sddm kwin konsole xorg-x11-server-Xwayland openssh-server xterm sudo webkit2gtk4.1 gtk3 alsa-lib libXtst libXrandr libXi libXcursor
id tester >/dev/null 2>&1 || useradd -m -s /bin/bash -G wheel tester
install -d -m 700 -o tester -g tester /home/tester/.ssh
install -m 600 -o tester -g tester /root/vmctl-authorized-key /home/tester/.ssh/authorized_keys
printf 'tester ALL=(ALL) NOPASSWD: ALL\n' > /etc/sudoers.d/vmctl-tester
chmod 440 /etc/sudoers.d/vmctl-tester
mkdir -p /etc/sddm.conf.d
cat > /etc/sddm.conf.d/50-vmctl.conf <<'EOF'
[Autologin]
User=tester
Session=plasma.desktop
EOF
touch /etc/cloud/cloud-init.disabled
ssh-keygen -A
mkdir -p /etc/NetworkManager/system-connections
cat > /etc/NetworkManager/system-connections/vmctl.nmconnection <<'EOF'
[connection]
id=vmctl
uuid=164f771b-62e6-4a41-a120-5e43b23e2cf0
type=ethernet
autoconnect=true
[ethernet]
[ipv4]
method=auto
[ipv6]
method=auto
EOF
chmod 600 /etc/NetworkManager/system-connections/vmctl.nmconnection
systemctl enable NetworkManager sshd sddm
systemctl set-default graphical.target
mkdir -p /home/tester/.config/autostart
cat > /home/tester/.config/autostart/vmctl-fingerprint.desktop <<'EOF'
[Desktop Entry]
Type=Application
Name=VM SSH fingerprint
Exec=xterm -geometry 100x12+50+50 -hold -e sh -c "ssh-keygen -lf /etc/ssh/ssh_host_ed25519_key.pub; echo VMCTL-FEDORA-44-PLASMA"
EOF
chown -R tester:tester /home/tester/.config
localectl set-keymap gb || true
