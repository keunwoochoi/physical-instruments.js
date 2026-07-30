// No asset/resource rule for .wasm and no experiments flag, on purpose. webpack 5
// understands `new URL("./x.wasm", import.meta.url)` natively; if this fixture needed a
// loader to make it work, that would be a finding about the library, not a fix.
const path = require("path");
const HtmlWebpackPlugin = require("html-webpack-plugin");
module.exports = {
  entry: "./main.js",
  output: { path: path.resolve(__dirname, "dist"), clean: true },
  plugins: [new HtmlWebpackPlugin({ title: "webpack fixture" })],
};
