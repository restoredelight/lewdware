#!/bin/sh
set -eu
dnf install -y NetworkManager sway sway-config-fedora swaybg waybar foot xorg-x11-server-Xwayland openssh-server sudo dbus-tools dbus-daemon alsa-lib libXtst libXrandr libXi libXcursor xdg-desktop-portal-wlr xdg-desktop-portal-gtk
id tester >/dev/null 2>&1 || useradd -m -s /bin/bash -G wheel tester
install -d -m 700 -o tester -g tester /home/tester/.ssh
install -m 600 -o tester -g tester /root/vmctl-authorized-key /home/tester/.ssh/authorized_keys
printf 'tester ALL=(ALL) NOPASSWD: ALL\n' > /etc/sudoers.d/vmctl-tester
chmod 440 /etc/sudoers.d/vmctl-tester
touch /etc/cloud/cloud-init.disabled
ssh-keygen -A
mkdir -p /etc/NetworkManager/system-connections
cat > /etc/NetworkManager/system-connections/vmctl.nmconnection <<'EOF'
[connection]
id=vmctl
uuid=c8af7d35-906e-46db-9a0d-573d73ff433d
type=ethernet
autoconnect=true
[ethernet]
[ipv4]
method=auto
[ipv6]
method=auto
EOF
chmod 600 /etc/NetworkManager/system-connections/vmctl.nmconnection
mkdir -p /etc/systemd/system/getty@tty1.service.d
cat > /etc/systemd/system/getty@tty1.service.d/autologin.conf <<'EOF'
[Service]
ExecStart=
ExecStart=-/sbin/agetty --autologin tester --noclear %I $TERM
EOF
cat >> /home/tester/.bash_profile <<'EOF'
if [ "$(tty)" = /dev/tty1 ] && [ -z "${WAYLAND_DISPLAY:-}" ]; then
  export XDG_CURRENT_DESKTOP=sway XDG_SESSION_TYPE=wayland
  exec sway
fi
EOF
# Retain Fedora's default Sway configuration and wallpaper; add only test access.
mkdir -p /home/tester/.config/sway
cat > /home/tester/.config/sway/config <<'EOF'
include /etc/sway/config
output * mode 1280x800
exec systemctl --user import-environment DISPLAY WAYLAND_DISPLAY SWAYSOCK XDG_CURRENT_DESKTOP XDG_SESSION_TYPE XDG_SESSION_ID
exec systemctl --user start graphical-session.target
exec foot sh -c 'ssh-keygen -lf /etc/ssh/ssh_host_ed25519_key.pub; echo VMCTL-FEDORA-44-SWAY; exec sleep infinity'
EOF
chown -R tester:tester /home/tester/.config /home/tester/.bash_profile
systemctl enable NetworkManager sshd getty@tty1
systemctl set-default multi-user.target
