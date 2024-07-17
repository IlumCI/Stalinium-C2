<template>
  <div>
    <h1>C2 Server Admin Panel</h1>
    <div v-for="command in commands" :key="command.id">
      <p>{{ command.result }}</p>
    </div>
  </div>
</template>

<script>
export default {
  data() {
    return {
      commands: [],
    };
  },
  methods: {
    executeCommand(command, args) {
      this.$tauri.invoke('execute_command', { command, args }).then((result) => {
        this.commands.push(result);
      });
    },
  },
};
</script>
