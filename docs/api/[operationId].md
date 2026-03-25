---
aside: false
outline: false
---

<script setup lang="ts">
import { useRoute } from 'vitepress'

const { operationId } = useRoute().data.params
</script>

<OAOperation :operationId="operationId" />
