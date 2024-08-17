# transmission-rss

## installation and example usage

### from source

```sh
git clone git@github.com:syrflover/transmission-rss.git
cargo install --path ./transmission-rss

export TRANSMISSION_URL=http://localhost:9091/transmission/rpc
export CHANNELS_CONFIG_URL=https://raw.githubusercontent.com/syrflover/syrflover/master/transmission-rss-channels.yaml

# run for one time
transmission-rss
```

## configuration

[example](https://github.com/syrflover/syrflover/blob/master/transmission-rss-channels.yaml)
