# QOI-rs
An implementation of the QOI image codec described [here](https://phoboslab.org/log/2021/11/qoi-fast-lossless-image-compression) and heavily based off of the reference implementation [here](https://github.com/phoboslab/qoi). 

## TODO
- [x] Encoder
- [ ] Decoder
- [ ] Test suite
- [ ] Benchmark suite
- [ ] Better error codes

## Ideas for an improved version of the file
* Include channel count in the header
* Support for channel counts <3 (or >4?)
* Tiles?