FROM alpine:3.22
RUN apk add --no-cache bash git ncurses \
    && touch /wtree-demo-container
COPY wt /usr/local/bin/wt
COPY setup.sh /opt/demo/setup.sh
COPY start.sh /opt/demo/start.sh
ENTRYPOINT ["/bin/bash", "/opt/demo/start.sh"]
