when debugging a bot's execution with gdb, you will run into difficulties around dynamic loading of the bot's .so

first set a catchpoint to interrupt the dynamic loader
```
catch load
run ssb bot:0 
```

this catches 2x, first for the sim loading, then for bot:0 loading.
after the bot has loaded, set your breakpoint in the bot's code

break wtfbot::Bot::setup
