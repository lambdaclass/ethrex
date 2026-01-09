async function main(){
for(let i = 22830000; i > 22800000; i--) {
var myHeaders = new Headers();
myHeaders.append("Content-Type", "application/json");

var raw = JSON.stringify({
  "method": "eth_getBlockByNumber",
  "params": [
    "0x"+i.toString(16),
    false
  ],
  "id": 1,
  "jsonrpc": "2.0"
});

var requestOptions = {
  method: 'POST',
  headers: myHeaders,
  body: raw,
  redirect: 'follow'
};

let p = fetch("https://mainnet.infura.io/v3/1f3215422eb64555b610ca5a96671d37", requestOptions)
  .then(response => response.text())
  .then(result => {
     console.log(i, JSON.parse(result).result.stateRoot);
  })
  .catch(error => console.log('error', error));
await p;
}
}

main()
