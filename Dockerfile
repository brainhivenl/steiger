FROM scratch

WORKDIR /bin

COPY target/release/steiger /bin

ENTRYPOINT ["/bin/steiger"]
