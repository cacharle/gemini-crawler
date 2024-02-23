

## TODO

- [x] asyncio to make it faster
    Note: asyncio with recursion is tricky, asyncio with shared references is tricky tricky
- [ ] Simpler graph that only contains domain names instead of individual capsules (pages)
- [ ] Throttle the number of connections
    - [ ] Throttle according to IP address
- [x] Add timeout to tcpstream connect and read/write
- [x] Serialize graph
    - [ ] Serialize visited and url_node_ids aswell
- [ ] Save gemtext next to the url in the url_node_id map
- [ ] check if error in gemini response (parse gemini reponse header)
