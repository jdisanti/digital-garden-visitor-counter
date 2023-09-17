# Digital Garden Visitor Counter

This repo holds a reusable visitor counter for use with a digital garden, or for other personal websites.
It functions as an AWS Lambda Funtion URL that renders a PNG image while also incrementing a counter
in a DynamoDB table. Multiple counters are supported, and differentiated by name. Visits are best-effort
deduplicated by keeping track of the hashes of IP and user agent, and the time that the visit occurred.

The font in the generated counter image is intentionally pixelated, and is supposed to resemble
old hit counters from 90s and early 2000s websites (think GeoCities). The image can be styled
with CSS to get different colors, borders, background colors, etc.

As an example, this README has been visited this many times:
![visit count](https://u3u6op73cfwfucgfi4lyfeusfa0gsndu.lambda-url.us-west-2.on.aws/?name=repo-readme)

Note: The above count doesn't benefit from deduplication since GitHub is proxying it through Camo, which
anonymizes the request, and thus, frequently increments the count. That is why you'll see the number increase
if you refresh the page when there are no other visitors. When viewing it
[outside of GitHub](https://u3u6op73cfwfucgfi4lyfeusfa0gsndu.lambda-url.us-west-2.on.aws/?name=repo-readme),
it will successfully deduplicate.

## Required tools for building

The following are needed to build and deploy this Lambda:
- [Rust](https://rustup.rs/)
- [NodeJS (18.x or later)](https://nodejs.org/)
- [Zig](https://ziglang.org/)
- [Just](https://crates.io/crates/just) (`cargo install --locked just` after installing Rust)
- [Cargo Lambda](https://www.cargo-lambda.info/guide/installation.html)

## Building

The Lambda can be tested and built with:
```
just test
just synth
```

## Deploying

1. Make sure your AWS CLI is authenticated with a default profile that you want to deploy with.
2. Run `just deploy`, or optionally, `just deploy <allowed-names> <min-width>` where `<allowed-names>` is
   a comma-delimited list of counter names to allow (the default is `default,repo-readme`), and `<min-width>` is
   the minimum width in number of digits to render the counter with (which defaults to '5').

If the deployment succeeds, it will print out the URL for the counter. For example:
```
Outputs:
digital-garden-visitor-counter.counterurloutput = https://{some-id}.lambda-url.us-west-2.on.aws/
```

To use this URL, simply embed it in an image tag:
```html
<img alt="visitor counter" src="https://{some-id}.lambda-url.us-west-2.on.aws/?name={name}">
```
Where `{name}` should be the name of the counter you want to display and increment, which needs
to match one of the allowed names in the `<allowed-names>` parameter above.

## Contributing

Contributions are welcome. For larger contributions, it's a good idea to create an issue to
discuss it before spending a lot of time and opening a pull request. You're also welcome to just
fork this repository to make your modifications without contributing, so long as the fork is
also open source.

## License

This project is licensed under the GPL-3.0. Any changes made to it must be open sourced,
and you can deploy it and use it on your website as you please.
