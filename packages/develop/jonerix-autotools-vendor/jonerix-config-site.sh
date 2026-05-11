# /etc/profile.d/jonerix-config-site.sh — wire CONFIG_SITE for autotools.
if [ -z "${CONFIG_SITE-}" ] && [ -r /etc/jonerix-config.site ]; then
    export CONFIG_SITE=/etc/jonerix-config.site
fi
