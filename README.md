

## TODO

- [x] asyncio to make it faster
    Note: asyncio with recursion is tricky, asyncio with shared references is tricky tricky
- [ ] Simpler graph that only contains domain names instead of individual capsules (pages)
- [x] Throttle the number of connections (sleep throttle, maybe using interval is better)
    - [ ] Throttle according to IP address
- [x] Add timeout to tcpstream connect and read/write
- [x] Serialize graph
    - [ ] Serialize visited and url_node_ids aswell
- [ ] Save gemtext next to the url in the url_node_id map
- [x] Check if error in gemini response (parse gemini reponse header)
- [x] Follow redirects
- [ ] TLS verify to true
- [ ] progress bar with nb visit/second (with error logs on top)
