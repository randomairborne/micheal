FROM alpine
ARG TARGETARCH

COPY /${TARGETARCH}-executables/micheal /usr/bin/
COPY /templates/ /etc/micheal/templates/

WORKDIR "/etc/micheal/"
ENTRYPOINT "/usr/bin/micheal"
