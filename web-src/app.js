import { highlightAll } from './3rd-party/speed-highlight.js'

const example = window.location.hash.substr(1) || "cells"

import(`./wasm/examples/${example}.js`).then((module) => {
  module.default()
}).catch(err => {
// impossible?!
  window.alert(err)
})

document.querySelector(`a[href="#${example}"]`).classList.add("active")

fetch(`./sources/${example}.rs`)
  .then(res => res.text())
  .then(text => {
    document.querySelector('#code').textContent = text;
    highlightAll()
  })

window.onhashchange = () => {
  window.location.reload()
}
