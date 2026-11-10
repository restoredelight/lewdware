#!/bin/sh
set -eu
export DEBIAN_FRONTEND=noninteractive
apt-get update
apt-get install -y xfce4 lightdm xserver-xorg xterm openssh-server dbus-x11 sudo libwebkit2gtk-4.1-0 libayatana-appindicator3-1 libxdo3 libasound2t64
id tester >/dev/null 2>&1 || useradd -m -s /bin/bash -G sudo tester
install -d -m 700 -o tester -g tester /home/tester/.ssh
install -m 600 -o tester -g tester /root/vmctl-authorized-key /home/tester/.ssh/authorized_keys
printf 'tester ALL=(ALL) NOPASSWD: ALL\n' > /etc/sudoers.d/vmctl-tester
chmod 440 /etc/sudoers.d/vmctl-tester
mkdir -p /etc/lightdm/lightdm.conf.d
cat > /etc/lightdm/lightdm.conf.d/50-vmctl.conf <<'EOF'
[Seat:*]
autologin-user=tester
autologin-user-timeout=0
user-session=xfce
EOF
# Cloud-init must not change this deliberately prepared local VM on first boot.
touch /etc/cloud/cloud-init.disabled
ssh-keygen -A
systemctl enable ssh lightdm
systemctl set-default graphical.target
mkdir -p /home/tester/.config/autostart
cat > /home/tester/.config/autostart/vmctl-fingerprint.desktop <<'EOF'
[Desktop Entry]
Type=Application
Name=VM SSH fingerprint
Exec=xterm -geometry 100x12+50+50 -hold -e sh -c "ssh-keygen -lf /etc/ssh/ssh_host_ed25519_key.pub; echo VMCTL-DEBIAN-13-XFCE"
EOF
chown -R tester:tester /home/tester/.config
printf 'XKBLAYOUT="gb"\n' > /etc/default/keyboard
