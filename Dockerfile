FROM scratch

WORKDIR /bin

COPY steiger /bin/steiger

ENTRYPOINT ["/bin/steiger"]
