## Project to make an IoT thermostat with an ESP32

### target features:
* read temperature frequently to collect data on room average room temperature throughout the day
* activate/deactivate HVAC control line in order to obtain goals, which include:
    - comfort
    - energy costs
* have a web interface that allows someone to set temperature targets throughout the day


### current design:
* [esp32c3 dev kit](https://www.amazon.com/gp/product/B09D3S4RPZ/ref=ppx_yo_dt_b_search_asin_title?ie=UTF8&th=1) as the PCB and MCU
* [thermostat](https://www.amazon.com/gp/product/B08V93CTM2/ref=ppx_yo_dt_b_search_asin_title?ie=UTF8&psc=1), connects via GPIO to esp32
* [relay](https://www.amazon.com/gp/product/B08W3XDNGK/ref=ppx_yo_dt_b_search_asin_title?ie=UTF8&th=1), connects via GPIO to esp32, connects to control line
* for power, can connect via USB, would need to think of how to run the line
* potentially could use battery, but I can see that getting tedious over time
* 3D print enclosure; TODO: find a design that will work with the current hole, possibly make one
* use low power mode to reduce power usage
* the best design would be some sort of local hub. I think using some sort of linux server would be best; can start with my raspberry pi TODO: add a link with info to the model and docs
* use esp-idf with rust std crate. increases size of files, but can code much faster
